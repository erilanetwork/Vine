use wasmtime::component::bindgen;

bindgen!({
    path: "../vine-plugin-wit/wit",
    world: "plugin",
    imports: {
        default: async | trappable,
    },
    exports: {
        default: async | store | trappable,
    },
});
