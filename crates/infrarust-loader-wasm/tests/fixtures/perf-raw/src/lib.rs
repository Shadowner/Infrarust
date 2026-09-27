wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

struct Component;

fixture_common::raw_fixture!(
    Component,
    id: "perf-raw",
    name: "Perf Raw Fixture",
    description: None,
    on_enable: {
        Ok(())
    }
);

export!(Component);
