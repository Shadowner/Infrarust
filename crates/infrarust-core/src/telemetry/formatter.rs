use std::fmt;

use console::Term;
use tracing::Level;
use tracing::field::{Field as TracingField, Visit};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::registry::LookupSpan;

use crate::terminal::{Tone, color};

const LEVEL_WIDTH: usize = 5;
const FIELD_GAP: &str = "  ";

pub struct InfrarustFormatter {
    colored: bool,
    width: Option<usize>,
}

impl InfrarustFormatter {
    pub fn new() -> Self {
        Self::with(color::enabled(), terminal_width())
    }

    pub const fn with(colored: bool, terminal_width: Option<usize>) -> Self {
        Self {
            colored,
            width: terminal_width,
        }
    }

    const fn interactive(&self) -> bool {
        self.width.is_some()
    }

    fn timestamp(&self) -> String {
        if self.interactive() {
            chrono::Local::now().format("%H:%M:%S").to_string()
        } else {
            humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string()
        }
    }

    fn render(&self, record: &Record<'_>) -> String {
        let head = self.render_head(record);
        let fields: Vec<Rendered> = record
            .fields
            .iter()
            .map(|field| field.render(self.colored))
            .collect();
        if fields.is_empty() {
            return head.text;
        }
        let inline = head.width + FIELD_GAP.len() + joined_width(&fields);
        match self.width {
            Some(width) if inline > width => {
                let indent = console::measure_text_width(record.time) + LEVEL_WIDTH + 2;
                let mut out = head.text;
                for line in pack(&fields, width.saturating_sub(indent)) {
                    out.push('\n');
                    out.push_str(&" ".repeat(indent));
                    out.push_str(&line);
                }
                out
            }
            _ => {
                let joined: Vec<&str> = fields.iter().map(|field| field.text.as_str()).collect();
                format!("{}{FIELD_GAP}{}", head.text, joined.join(" "))
            }
        }
    }

    fn render_head(&self, record: &Record<'_>) -> Rendered {
        let text = format!(
            "{} {} {}",
            Tone::Muted.paint(record.time, self.colored),
            level_tone(record.level)
                .paint(level_label(record.level), self.colored)
                .bold(),
            message_tone(record.level).paint(record.message, self.colored),
        );
        Rendered {
            width: console::measure_text_width(&text),
            text,
        }
    }
}

impl Default for InfrarustFormatter {
    fn default() -> Self {
        Self::new()
    }
}

fn terminal_width() -> Option<usize> {
    let term = Term::stdout();
    if !term.is_term() {
        return None;
    }
    term.size_checked().map(|(_, columns)| usize::from(columns))
}

impl<S, N> FormatEvent<S, N> for InfrarustFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let mut fields = visitor.fields;
        append_span_context(ctx, &mut fields);
        let time = self.timestamp();
        let record = Record {
            time: &time,
            level: *event.metadata().level(),
            message: visitor.message.as_deref().unwrap_or_default(),
            fields: &fields,
        };
        writeln!(writer, "{}", self.render(&record))
    }
}

fn append_span_context<S, N>(ctx: &FmtContext<'_, S, N>, fields: &mut Vec<Field>)
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    let Some(scope) = ctx.event_scope() else {
        return;
    };
    let mut names = Vec::new();
    for span in scope.from_root() {
        names.push(span.name());
        if let Some(recorded) = span.extensions().get::<FormattedFields<N>>() {
            merge(
                fields,
                parse_span_fields(&console::strip_ansi_codes(recorded)),
            );
        }
    }
    if !names.is_empty() {
        fields.push(Field::pair("span", names.join(">")));
    }
}

fn merge(fields: &mut Vec<Field>, extra: Vec<Field>) {
    for field in extra {
        if !fields.iter().any(|known| known.key == field.key) {
            fields.push(field);
        }
    }
}

fn parse_span_fields(text: &str) -> Vec<Field> {
    tokens(text)
        .into_iter()
        .map(|token| match token.split_once('=') {
            Some((key, value)) => Field::pair(key, unquote(value)),
            None => Field::bare(token),
        })
        .collect()
}

fn tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut quoted = false;
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if c.is_whitespace() && !quoted {
            if let Some(from) = start.take() {
                tokens.push(&text[from..index]);
            }
            continue;
        }
        start.get_or_insert(index);
        if escaped {
            escaped = false;
        } else if c == '\\' && quoted {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        }
    }
    if let Some(from) = start {
        tokens.push(&text[from..]);
    }
    tokens
}

fn unquote(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some(next @ ('"' | '\\'))) => {
                out.push(next);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

struct Record<'a> {
    time: &'a str,
    level: Level,
    message: &'a str,
    fields: &'a [Field],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    key: String,
    value: Option<String>,
}

impl Field {
    fn pair(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: Some(value.into()),
        }
    }

    fn bare(token: impl Into<String>) -> Self {
        Self {
            key: token.into(),
            value: None,
        }
    }

    fn render(&self, colored: bool) -> Rendered {
        let Some(value) = &self.value else {
            return Rendered {
                width: console::measure_text_width(&self.key),
                text: self.key.clone(),
            };
        };
        let value = quote(value);
        let width =
            console::measure_text_width(&self.key) + 1 + console::measure_text_width(&value);
        let text = format!(
            "{}{}{value}",
            Tone::Muted.paint(&self.key, colored),
            Tone::Muted.paint("=", colored),
        );
        Rendered { width, text }
    }
}

struct Rendered {
    width: usize,
    text: String,
}

fn quote(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c == '=' || c == '"');
    if needs_quotes {
        format!("{value:?}")
    } else {
        value.to_string()
    }
}

fn joined_width(fields: &[Rendered]) -> usize {
    fields.iter().map(|field| field.width).sum::<usize>() + fields.len().saturating_sub(1)
}

fn pack(fields: &[Rendered], room: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for field in fields {
        if used > 0 && used + 1 + field.width > room {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if used > 0 {
            line.push(' ');
            used += 1;
        }
        line.push_str(&field.text);
        used += field.width;
    }
    if used > 0 {
        lines.push(line);
    }
    lines
}

const fn level_label(level: Level) -> &'static str {
    match level {
        Level::TRACE => "TRACE",
        Level::DEBUG => "DEBUG",
        Level::INFO => "INFO ",
        Level::WARN => "WARN ",
        Level::ERROR => "ERROR",
    }
}

const fn level_tone(level: Level) -> Tone {
    match level {
        Level::TRACE => Tone::Muted,
        Level::DEBUG => Tone::Entity,
        Level::INFO => Tone::Ok,
        Level::WARN => Tone::Warn,
        Level::ERROR => Tone::Err,
    }
}

const fn message_tone(level: Level) -> Tone {
    match level {
        Level::WARN => Tone::Warn,
        Level::ERROR => Tone::Err,
        _ => Tone::Plain,
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<Field>,
}

impl MessageVisitor {
    fn push(&mut self, field: &TracingField, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push(Field::pair(field.name(), value));
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &TracingField, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        let value = rendered
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .map_or_else(|| rendered.clone(), str::to_string);
        self.push(field, value);
    }

    fn record_str(&mut self, field: &TracingField, value: &str) {
        self.push(field, value.to_string());
    }

    fn record_i64(&mut self, field: &TracingField, value: i64) {
        self.push(field, value.to_string());
    }

    fn record_u64(&mut self, field: &TracingField, value: u64) {
        self.push(field, value.to_string());
    }

    fn record_bool(&mut self, field: &TracingField, value: bool) {
        self.push(field, value.to_string());
    }

    fn record_f64(&mut self, field: &TracingField, value: f64) {
        self.push(field, value.to_string());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing::Level;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    use super::{Field, InfrarustFormatter, Record, merge, parse_span_fields};

    fn render(fields: &[Field], colored: bool, width: Option<usize>) -> String {
        render_with(Level::INFO, "player joined", fields, colored, width)
    }

    fn render_with(
        level: Level,
        message: &str,
        fields: &[Field],
        colored: bool,
        width: Option<usize>,
    ) -> String {
        InfrarustFormatter::with(colored, width).render(&Record {
            time: "12:00:00",
            level,
            message,
            fields,
        })
    }

    fn many_fields() -> Vec<Field> {
        vec![
            Field::pair("player", "Notch"),
            Field::pair("uuid", "069a79f4-44e9-4726-a5be-fca90e38aaf5"),
            Field::pair("server", "lobby"),
            Field::pair("addr", "203.0.113.7:51234"),
            Field::pair("protocol", "772"),
            Field::pair("mode", "client_only"),
        ]
    }

    #[test]
    fn an_event_renders_on_a_single_logfmt_line() {
        let line = render(
            &[
                Field::pair("player", "Notch"),
                Field::pair("server", "lobby"),
            ],
            false,
            None,
        );
        assert_eq!(
            line,
            "12:00:00 INFO  player joined  player=Notch server=lobby"
        );
    }

    #[test]
    fn an_event_without_fields_has_no_trailing_space() {
        let line = render_with(Level::ERROR, "boom", &[], false, None);
        assert_eq!(line, "12:00:00 ERROR boom");
    }

    #[test]
    fn values_that_would_break_logfmt_are_quoted() {
        let line = render(
            &[
                Field::pair("reason", "timed out"),
                Field::pair("empty", ""),
                Field::pair("expr", "a=b"),
                Field::pair("said", "\"hi\""),
                Field::pair("plain", "ok"),
            ],
            false,
            None,
        );
        assert_eq!(
            line,
            r#"12:00:00 INFO  player joined  reason="timed out" empty="" expr="a=b" said="\"hi\"" plain=ok"#
        );
    }

    #[test]
    fn a_long_line_wraps_its_fields_under_the_message() {
        let line = render(&many_fields(), false, Some(80));
        let expected = [
            "12:00:00 INFO  player joined",
            "               player=Notch uuid=069a79f4-44e9-4726-a5be-fca90e38aaf5",
            "               server=lobby addr=203.0.113.7:51234 protocol=772 mode=client_only",
        ]
        .join("\n");
        assert_eq!(line, expected);
        assert!(line.lines().all(|row| row.len() <= 80), "{line}");
    }

    #[test]
    fn a_field_wider_than_the_room_sits_alone_on_its_line() {
        let long = "x".repeat(70);
        let line = render(
            &[
                Field::pair("a", "1"),
                Field::pair("blob", long.clone()),
                Field::pair("b", "2"),
            ],
            false,
            Some(40),
        );
        let rows: Vec<&str> = line.lines().collect();
        assert_eq!(rows.len(), 4, "{line}");
        assert_eq!(rows[1].trim(), "a=1");
        assert_eq!(rows[2].trim(), format!("blob={long}"));
        assert_eq!(rows[3].trim(), "b=2");
    }

    #[test]
    fn a_line_that_fits_is_not_wrapped() {
        let line = render(&[Field::pair("player", "Notch")], false, Some(80));
        assert_eq!(line, "12:00:00 INFO  player joined  player=Notch");
    }

    #[test]
    fn without_a_width_the_event_stays_on_one_line() {
        let line = render(&many_fields(), false, None);
        assert_eq!(line.lines().count(), 1);
        assert!(line.ends_with("mode=client_only"), "{line}");
    }

    #[test]
    fn colored_output_strips_back_to_the_plain_one() {
        for level in [
            Level::TRACE,
            Level::DEBUG,
            Level::INFO,
            Level::WARN,
            Level::ERROR,
        ] {
            for width in [None, Some(80)] {
                let plain = render_with(level, "hello there", &many_fields(), false, width);
                let colored = render_with(level, "hello there", &many_fields(), true, width);
                assert_ne!(plain, colored);
                assert_eq!(console::strip_ansi_codes(&colored), plain);
            }
        }
    }

    #[test]
    fn span_fields_are_read_token_by_token() {
        let parsed =
            parse_span_fields(r#"server="lobby" reason="timed out" said="a \"b\"" id=7 flag"#);
        assert_eq!(
            parsed,
            vec![
                Field::pair("server", "lobby"),
                Field::pair("reason", "timed out"),
                Field::pair("said", "a \"b\""),
                Field::pair("id", "7"),
                Field::bare("flag"),
            ]
        );
    }

    #[test]
    fn a_bare_span_token_renders_as_is() {
        let line = render(&[Field::bare("flag")], false, None);
        assert_eq!(line, "12:00:00 INFO  player joined  flag");
    }

    #[test]
    fn event_fields_win_over_span_fields() {
        let mut fields = vec![Field::pair("server", "survival")];
        merge(
            &mut fields,
            vec![
                Field::pair("server", "lobby"),
                Field::pair("player", "Notch"),
            ],
        );
        assert_eq!(
            fields,
            vec![
                Field::pair("server", "survival"),
                Field::pair("player", "Notch")
            ]
        );
    }

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Buffer {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

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

    fn capture(emit: impl FnOnce()) -> String {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .event_format(InfrarustFormatter::with(false, None))
                .with_writer(buffer.clone()),
        );
        tracing::subscriber::with_default(subscriber, emit);
        buffer.contents()
    }

    #[test]
    fn a_real_subscriber_writes_the_event_and_its_span_chain() {
        let out = capture(|| {
            let session = tracing::info_span!("session");
            let _session = session.enter();
            let login = tracing::info_span!("login");
            let _login = login.enter();
            tracing::warn!(player = "Notch", "slow login");
        });
        assert_eq!(out.lines().count(), 1, "{out}");
        assert!(
            out.trim_end()
                .ends_with("WARN  slow login  player=Notch span=session>login"),
            "{out}"
        );
    }

    #[test]
    fn a_real_subscriber_appends_span_fields_after_the_event_ones() {
        let out = capture(|| {
            let session =
                tracing::info_span!("session", player = "Steve", addr = "203.0.113.7:51234");
            let _session = session.enter();
            let login = tracing::info_span!("login", server = "lobby", reason = "first join");
            let _login = login.enter();
            tracing::info!(player = "Notch", "connected");
        });
        assert!(
            out.trim_end().ends_with(
                r#"INFO  connected  player=Notch addr=203.0.113.7:51234 server=lobby reason="first join" span=session>login"#
            ),
            "{out}"
        );
    }
}
