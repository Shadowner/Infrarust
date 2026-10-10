wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

struct Component;

fixture_common::raw_fixture_with_metadata!(
    Component,
    metadata: {
        panic!("metadata-trap: metadata() panics on purpose");
    },
    on_enable: {
        Ok(())
    }
);

export!(Component);
