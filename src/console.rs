use std::borrow::Cow;
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, EditMode, Editor, ExternalPrinter, Helper};
use tokio::sync::broadcast;
use tracing::{error, info};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use vine_plugin::command::{CommandDispatcher, ProxyCommandSource};

type ConsoleEditor = Editor<CommandCompleter, DefaultHistory>;

/// Owns the interactive line editor until the proxy is ready to read commands. Without a
/// terminal on stdin it falls back to plain line reads, so piped input and service managers
/// keep working.
pub struct Console {
    editor: Option<ConsoleEditor>,
}

/// Sets up logging and, when stdin is a terminal, the line editor logs print around.
/// Call it once, before anything logs.
pub fn init() -> Console {
    let mut editor = io::stdin()
        .is_terminal()
        .then(|| {
            let config = Config::builder()
                .auto_add_history(true)
                .completion_type(CompletionType::List)
                .edit_mode(EditMode::Emacs)
                .build();
            Editor::with_config(config).ok()
        })
        .flatten();

    let printer = editor
        .as_mut()
        .and_then(|rl| rl.create_external_printer().ok())
        .map(|p| Box::new(p) as Box<dyn ExternalPrinter + Send>);

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(ConsoleWriter::new(printer)))
                .with_timer(ChronoLocal::new("%H:%M:%S".to_string()))
                .with_target(false)
                // yeah, command feedback carries its own colors, escaping them prints \x1b soup
                .with_ansi_sanitization(false),
        )
        .init();

    Console { editor }
}

impl Console {
    /// Starts reading console commands on their own thread. `stop` and `end` shut the proxy
    /// down, anything else goes to the dispatcher.
    pub fn spawn(
        self,
        dispatcher: Arc<CommandDispatcher>,
        source: ProxyCommandSource,
        shutdown_tx: broadcast::Sender<()>,
    ) {
        let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let runtime = tokio::runtime::Handle::current();

        let reader = match self.editor {
            Some(mut rl) => {
                rl.set_helper(Some(CommandCompleter {
                    dispatcher: dispatcher.clone(),
                    source: source.clone(),
                }));
                let shutdown_tx = shutdown_tx.clone();
                std::thread::Builder::new()
                    .name("console".to_string())
                    .spawn(move || {
                        // plugin suggestions block_on the runtime, so this thread has to be inside it
                        let _runtime = runtime.enter();
                        read_editor(rl, line_tx, shutdown_tx);
                    })
            }
            None => std::thread::Builder::new()
                .name("console".to_string())
                .spawn(move || read_stdin(line_tx)),
        };
        if let Err(e) = reader {
            error!("Failed to start the console: {e}");
            return;
        }

        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    line = line_rx.recv() => match line {
                        Some(line) if is_stop(&line) => {
                            info!("Shutting down (console)");
                            let _ = shutdown_tx.send(());
                        }
                        Some(line) => dispatcher.handle_command(&source, &line),
                        None => break,
                    },
                }
            }
        });
    }
}

fn is_stop(line: &str) -> bool {
    line.eq_ignore_ascii_case("stop") || line.eq_ignore_ascii_case("end")
}

fn read_editor(
    mut rl: ConsoleEditor,
    line_tx: tokio::sync::mpsc::UnboundedSender<String>,
    shutdown_tx: broadcast::Sender<()>,
) {
    loop {
        match rl.readline("> ") {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                // ok so stopping has to end this loop too, a process that exits inside readline
                // leaves the terminal in raw mode
                let stop = is_stop(&line);
                if line_tx.send(line).is_err() || stop {
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => {
                info!("Shutting down (Ctrl+C)");
                let _ = shutdown_tx.send(());
                break;
            }
            Err(ReadlineError::Eof) => {
                info!("Shutting down (Ctrl+D)");
                let _ = shutdown_tx.send(());
                break;
            }
            Err(e) => {
                error!("Failed to read console input: {e}");
                break;
            }
        }
    }
}

fn read_stdin(line_tx: tokio::sync::mpsc::UnboundedSender<String>) {
    let stdin = io::stdin();
    let mut line = String::new();
    while let Ok(n) = stdin.read_line(&mut line) {
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() && line_tx.send(trimmed.to_string()).is_err() {
            break;
        }
        line.clear();
    }
}

/// Routes log lines through the editor so they land above the prompt instead of through it.
struct ConsoleWriter {
    printer: Option<Box<dyn ExternalPrinter + Send>>,
    buffer: Vec<u8>,
}

impl ConsoleWriter {
    fn new(printer: Option<Box<dyn ExternalPrinter + Send>>) -> Self {
        Self {
            printer,
            buffer: Vec::new(),
        }
    }
}

impl Write for ConsoleWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(printer) = self.printer.as_mut() else {
            return io::stdout().write(buf);
        };
        // hmm, the fmt layer may hand over a line in pieces, so only whole lines go to the printer
        self.buffer.extend_from_slice(buf);
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            if printer
                .print(String::from_utf8_lossy(&line).into_owned())
                .is_err()
            {
                let mut stdout = io::stdout().lock();
                let _ = stdout.write_all(&line);
                let _ = stdout.flush();
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.printer.is_none() {
            return io::stdout().flush();
        }
        Ok(())
    }
}

/// Tab completion, inline hints and command highlighting for the console, all driven by the
/// same dispatcher players get their suggestions from.
struct CommandCompleter {
    dispatcher: Arc<CommandDispatcher>,
    source: ProxyCommandSource,
}

impl Helper for CommandCompleter {}
impl Validator for CommandCompleter {}

impl Completer for CommandCompleter {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let input = &line[..pos];
        let slash = usize::from(input.starts_with('/'));
        let suggestions = self.dispatcher.suggest_with_range(input, &self.source);
        let mut candidates: Vec<String> = suggestions
            .suggestions
            .iter()
            .map(|s| s.text.cached_text().clone())
            .collect();

        // stop lives here, not in the dispatcher, but it should still complete
        let root = &input[slash..];
        if !root.contains(' ') && "stop".starts_with(&root.to_ascii_lowercase()) {
            candidates.push("stop".to_string());
            candidates.sort();
        }
        Ok((suggestions.range.start + slash, candidates))
    }
}

impl Hinter for CommandCompleter {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        if line.is_empty() || pos < line.len() {
            return None;
        }
        let (start, candidates) = self.complete(line, pos, ctx).ok()?;
        let typed = line.get(start..)?;
        candidates
            .first()?
            .strip_prefix(typed)
            .filter(|rest| !rest.is_empty())
            .map(str::to_string)
    }
}

impl Highlighter for CommandCompleter {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        let end = line.find(' ').unwrap_or(line.len());
        let (cmd, args) = line.split_at(end);
        Cow::Owned(format!("\x1b[1;36m{cmd}\x1b[0m{args}"))
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        true
    }
}
