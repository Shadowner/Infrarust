wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

struct Component;

fixture_common::raw_fixture_with_metadata!(
    Component,
    metadata: {
        std::thread::sleep(std::time::Duration::from_secs(3600));
        crate::exports::infrarust::plugin::guest::PluginMetadata {
            id: ::std::string::String::from("metadata-sleep"),
            name: ::std::string::String::from("Metadata Sleep Fixture"),
            version: ::std::string::String::from("0.1.0"),
            authors: ::std::vec::Vec::new(),
            description: None,
            dependencies: ::std::vec::Vec::new(),
        }
    },
    on_enable: {
        Ok(())
    }
);

export!(Component);
