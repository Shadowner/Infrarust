use infrarust_api::types::Component;

use crate::bindings::infrarust::plugin::{text, types as wt};
use crate::component;
use crate::config::SandboxLimits;
use crate::store_state::build_probe_state;

use super::text_of;

#[tokio::test]
async fn the_text_interface_uses_the_native_parser_and_serializer() {
    let mut state = build_probe_state("probe".to_owned(), &SandboxLimits::default());
    let parsed = text::Host::parse_json(&mut state, r#"{"text":"hi","bold":true}"#.into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        component::from_wit(&parsed).unwrap(),
        Component::text("hi").bold()
    );
    let bad = text::Host::parse_json(&mut state, "{".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(bad.kind, wt::ErrorKind::InvalidArgument);
    let legacy = text::Host::parse_legacy(&mut state, "&aGreen".into())
        .await
        .unwrap();
    assert_eq!(
        text::Host::to_plain(&mut state, legacy).await.unwrap(),
        Ok("Green".to_owned())
    );
    let json = text::Host::to_json(&mut state, text_of("x")).await.unwrap();
    assert_eq!(json, Ok(Component::text("x").to_json()));
}
