#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io;
use std::sync::{Arc, Mutex};

use infrarust_config::OfflineUuidPolicy;
use infrarust_core::auth::game_profile::offline_profile;
use infrarust_core::console::output::{
    Block, CommandOutput, Failure, Hint, Line, OutputLine, Span, Table,
};
use infrarust_core::console::render::Renderer;
use infrarust_core::telemetry::formatter::InfrarustFormatter;
use infrarust_core::util::text::escape_control;
use infrarust_protocol::ProtocolVersion;
use infrarust_protocol::legacy::parse_legacy_handshake;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::login::SLoginStart;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

const HOSTILE: &str = "a\u{1b}[2Jb\nc\u{7}d\u{9b}e";

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn log_line(emit: impl FnOnce()) -> String {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .event_format(InfrarustFormatter::with(false, None))
            .with_writer(buffer.clone()),
    );
    tracing::subscriber::with_default(subscriber, emit);
    let bytes = buffer.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

fn assert_clean(rendered: &str) {
    assert!(!rendered.contains(char::is_control), "{rendered:?}");
}

#[test]
fn escaping_shows_every_control_character_and_keeps_the_rest() {
    assert_eq!(escape_control(HOSTILE), "a\\u{1b}[2Jb\\nc\\u{7}d\\u{9b}e");
    assert_eq!(escape_control("Ünï_名前"), "Ünï_名前");
}

#[test]
fn a_log_message_cannot_forge_a_line_or_drive_the_terminal() {
    let out = log_line(|| tracing::warn!("{HOSTILE}"));
    assert_clean(out.trim_end_matches('\n'));
    assert_eq!(out.lines().count(), 1, "{out:?}");
    assert!(out.contains("a\\u{1b}[2Jb\\nc"), "{out:?}");
}

#[test]
fn a_log_field_is_escaped_with_or_without_quotes() {
    let out = log_line(|| {
        tracing::info!(
            spaced = %format!("x y{HOSTILE}"),
            bare = %"p\u{1b}]0;owned\u{7}",
            c1 = %"p\u{9b}q",
            "joined"
        );
    });
    assert_clean(out.trim_end_matches('\n'));
    assert!(out.contains("bare=p\\u{1b}]0;owned\\u{7}"), "{out:?}");
    assert!(out.contains("c1=p\\u{9b}q"), "{out:?}");
}

#[test]
fn a_span_field_is_escaped_including_its_bare_tokens() {
    let out = log_line(|| {
        let span = tracing::info_span!("connection", player.username = %HOSTILE);
        let _span = span.enter();
        tracing::info!("connected");
    });
    assert_clean(out.trim_end_matches('\n'));
    assert_eq!(out.lines().count(), 1, "{out:?}");
}

#[test]
fn every_console_text_is_escaped() {
    let mut table = Table::new(&["Player"]);
    table.row([Line::from(Span::entity(HOSTILE))]);
    let block: CommandOutput = Block::new(HOSTILE).meta(HOSTILE).table(table).into();
    let outputs = [
        block,
        CommandOutput::Success(HOSTILE.to_string()),
        CommandOutput::Note(HOSTILE.to_string()),
        CommandOutput::Lines(vec![
            OutputLine::Success(HOSTILE.to_string()),
            OutputLine::Warning(HOSTILE.to_string()),
            OutputLine::Error(HOSTILE.to_string()),
        ]),
        CommandOutput::Error(Failure::new(HOSTILE).with_hint(Hint::Note(HOSTILE.to_string()))),
        CommandOutput::Error(Failure::new(HOSTILE).with_hint(Hint::Usage(HOSTILE.to_string()))),
    ];
    for output in &outputs {
        for width in [None, Some(12)] {
            let rendered = Renderer::new(false, width).render(output);
            for line in rendered.lines() {
                assert_clean(line);
            }
            assert!(rendered.contains("\\u{1b}"), "{rendered:?}");
        }
        let colored = Renderer::new(true, None).render(output);
        for line in console::strip_ansi_codes(&colored).lines() {
            assert_clean(line);
        }
    }
}

fn login_start(name: &str) -> Vec<u8> {
    let mut payload = vec![u8::try_from(name.len()).unwrap()];
    payload.extend_from_slice(name.as_bytes());
    payload
}

#[test]
fn a_login_start_with_a_control_character_is_refused_by_both_decoders() {
    for name in ["a\u{1b}b", "a\nb", "a\u{0}b", "a\u{7f}b", "a\u{9b}b"] {
        let payload = login_start(name);
        assert!(
            SLoginStart::decode(&mut &payload[..], ProtocolVersion::V1_8).is_err(),
            "{name:?}"
        );
        assert!(
            SLoginStart::decode_lenient(&mut &payload[..]).is_err(),
            "{name:?}"
        );
    }
}

#[test]
fn a_login_start_keeps_its_non_ascii_names() {
    for name in ["Notch", "joueur_é", "名前", "with space", "a.b-c"] {
        let payload = login_start(name);
        let decoded = SLoginStart::decode(&mut &payload[..], ProtocolVersion::V1_8).unwrap();
        assert_eq!(decoded.name, name);
    }
}

fn string16(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = u16::try_from(units.len()).unwrap().to_be_bytes().to_vec();
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn legacy_handshake(username: &str) -> Vec<u8> {
    let mut data = vec![78];
    data.extend(string16(username));
    data.extend(string16("mc.example.com"));
    data.extend_from_slice(&25565i32.to_be_bytes());
    data
}

fn pre_1_3_handshake(username: &str) -> Vec<u8> {
    let connection = string16(&format!("{username};mc.example.com:25565"));
    let mut data = vec![0x00];
    data.extend_from_slice(&connection[1..]);
    data
}

#[test]
fn a_legacy_username_with_a_control_character_is_refused() {
    for name in ["a\u{1b}b", "a\nb", "a\u{9b}b"] {
        assert!(parse_legacy_handshake(&legacy_handshake(name)).is_err());
        assert!(parse_legacy_handshake(&pre_1_3_handshake(name)).is_err());
    }
}

#[test]
fn a_legacy_username_keeps_its_non_ascii_form() {
    for handshake in [legacy_handshake("joueur_é"), pre_1_3_handshake("joueur_é")] {
        assert_eq!(
            parse_legacy_handshake(&handshake).unwrap().username,
            "joueur_é"
        );
    }
}

#[test]
fn an_offline_profile_refuses_a_control_character_under_every_policy() {
    for policy in [OfflineUuidPolicy::Offline, OfflineUuidPolicy::Client] {
        assert!(offline_profile(policy, "a\u{1b}b", None).is_err());
        assert_eq!(
            offline_profile(policy, "joueur_é", None).unwrap().username,
            "joueur_é"
        );
    }
}
