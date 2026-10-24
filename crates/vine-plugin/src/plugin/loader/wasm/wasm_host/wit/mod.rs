use wasmtime::component::bindgen;

bindgen!({
    path: "../../wit",
    world: "plugin",
    imports: {
        default: async | trappable,
    },
    exports: {
        default: async | store | trappable,
    },
});
