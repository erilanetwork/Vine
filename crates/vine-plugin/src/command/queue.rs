use pumpkin_command::argument_builder::{ArgumentBuilder, command, literal};
use pumpkin_command::context::command_context::CommandContext;
use pumpkin_command::dispatcher::CommandDispatcher as RawCommandDispatcher;
use pumpkin_command::node::{CommandExecutor, CommandExecutorResult};

use super::{CommandSender, CommandSyntaxError, ProxyCommandSource, command_error};

fn player_uuid(source: &ProxyCommandSource) -> Result<uuid::Uuid, CommandSyntaxError> {
    let CommandSender::Player { username, .. } = &source.sender else {
        return Err(command_error(source.tr("vine.queue.console", &[])));
    };
    source
        .session_manager
        .get_session(username)
        .map(|s| s.uuid)
        .ok_or_else(|| command_error(source.tr("vine.queue.none", &[])))
}

struct QueuePosition;

impl CommandExecutor<ProxyCommandSource> for QueuePosition {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let uuid = player_uuid(source)?;
        match source.services.queue.position(uuid) {
            Some((server, position)) => {
                let total = source.services.queue.len(&server);
                source.reply("vine.queue.status", &[&position, &total, &server]);
                Ok(position as i32)
            }
            None => Err(command_error(source.tr("vine.queue.none", &[]))),
        }
    }
}

struct QueueLeave;

impl CommandExecutor<ProxyCommandSource> for QueueLeave {
    fn execute(&self, context: &CommandContext<ProxyCommandSource>) -> CommandExecutorResult {
        let source = &context.source;
        let uuid = player_uuid(source)?;
        match source.services.queue.leave(uuid) {
            Some(server) => {
                source.reply("vine.queue.left", &[&server]);
                Ok(1)
            }
            None => Err(command_error(source.tr("vine.queue.none", &[]))),
        }
    }
}

pub fn register(dispatcher: &mut RawCommandDispatcher<ProxyCommandSource>) {
    dispatcher.register(
        command("queue", "Shows your place in a server queue, or leaves it")
            // no permission on purpose, it only ever touches the player's own place in line
            .executes(QueuePosition)
            .then(literal("leave").executes(QueueLeave)),
    );
}
