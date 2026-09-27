use std::fmt;

use crate::terminal::{Mark, Tone};

pub enum CommandOutput {
    Block(Block),
    Lines(Vec<OutputLine>),
    Success(String),
    Note(String),
    Error(Failure),
    None,
}

impl CommandOutput {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(Failure::new(message))
    }

    pub fn usage(usage: impl Into<String>) -> Self {
        Self::Error(Failure::new("Missing arguments").with_hint(Hint::Usage(usage.into())))
    }
}

impl From<Block> for CommandOutput {
    fn from(block: Block) -> Self {
        Self::Block(block)
    }
}

impl From<Failure> for CommandOutput {
    fn from(failure: Failure) -> Self {
        Self::Error(failure)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub hint: Option<Hint>,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: Hint) -> Self {
        self.hint = Some(hint);
        self
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    Usage(String),
    Note(String),
}

pub enum OutputLine {
    Success(String),
    Warning(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub tone: Tone,
    pub mark: Option<Mark>,
}

impl Span {
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
            mark: None,
        }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Plain)
    }

    pub fn strong(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Strong)
    }

    pub fn entity(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Entity)
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Muted)
    }

    pub fn ok(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Ok)
    }

    pub fn warn(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Warn)
    }

    pub fn err(text: impl Into<String>) -> Self {
        Self::new(text, Tone::Err)
    }

    pub fn marked(mark: Mark, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: mark.tone(),
            mark: Some(mark),
        }
    }
}

impl From<&str> for Span {
    fn from(text: &str) -> Self {
        Self::plain(text)
    }
}

impl From<String> for Span {
    fn from(text: String) -> Self {
        Self::plain(text)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line(pub Vec<Span>);

impl Line {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(mut self, span: impl Into<Span>) -> Self {
        self.0.push(span.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0
            .iter()
            .all(|span| span.text.is_empty() && span.mark.is_none())
    }
}

impl From<Span> for Line {
    fn from(span: Span) -> Self {
        Self(vec![span])
    }
}

impl From<Vec<Span>> for Line {
    fn from(spans: Vec<Span>) -> Self {
        Self(spans)
    }
}

impl From<&str> for Line {
    fn from(text: &str) -> Self {
        Span::plain(text).into()
    }
}

impl From<String> for Line {
    fn from(text: String) -> Self {
        Span::plain(text).into()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Line>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|header| (*header).to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn bare() -> Self {
        Self::default()
    }

    pub fn row<C: Into<Line>>(&mut self, cells: impl IntoIterator<Item = C>) -> &mut Self {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields(pub Vec<(String, Line)>);

impl Fields {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, label: impl Into<String>, value: impl Into<Line>) -> Self {
        self.0.push((label.into(), value.into()));
        self
    }

    pub fn push(&mut self, label: impl Into<String>, value: impl Into<Line>) {
        self.0.push((label.into(), value.into()));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Table(Table),
    Fields(Fields),
    Heading(String),
    Line(Line),
    Gap,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub title: String,
    pub meta: Line,
    pub body: Vec<Node>,
}

impl Block {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    pub fn meta(mut self, meta: impl Into<Line>) -> Self {
        self.meta = meta.into();
        self
    }

    pub fn table(mut self, table: Table) -> Self {
        self.body.push(Node::Table(table));
        self
    }

    pub fn fields(mut self, fields: Fields) -> Self {
        self.body.push(Node::Fields(fields));
        self
    }

    pub fn heading(mut self, text: impl Into<String>) -> Self {
        self.body.push(Node::Heading(text.into()));
        self
    }

    pub fn line(mut self, line: impl Into<Line>) -> Self {
        self.body.push(Node::Line(line.into()));
        self
    }

    pub fn gap(mut self) -> Self {
        self.body.push(Node::Gap);
        self
    }

    pub fn push(&mut self, node: Node) {
        self.body.push(node);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    Players,
    Bans,
    Servers,
    Config,
    Plugins,
    System,
}

impl CommandCategory {
    pub const ALL: [CommandCategory; 6] = [
        Self::Players,
        Self::Bans,
        Self::Servers,
        Self::Config,
        Self::Plugins,
        Self::System,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Players => "Players",
            Self::Bans => "Bans",
            Self::Servers => "Servers",
            Self::Config => "Configuration",
            Self::Plugins => "Plugins",
            Self::System => "System",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn test_command_category_display_names() {
        assert_eq!(CommandCategory::Players.display_name(), "Players");
        assert_eq!(CommandCategory::Bans.display_name(), "Bans");
        assert_eq!(CommandCategory::Servers.display_name(), "Servers");
        assert_eq!(CommandCategory::Config.display_name(), "Configuration");
        assert_eq!(CommandCategory::Plugins.display_name(), "Plugins");
        assert_eq!(CommandCategory::System.display_name(), "System");
    }

    #[test]
    fn usage_reports_missing_arguments_with_the_usage_as_hint() {
        match CommandOutput::usage("kick <player>") {
            CommandOutput::Error(failure) => {
                assert_eq!(failure.message, "Missing arguments");
                assert_eq!(failure.hint, Some(Hint::Usage("kick <player>".into())));
            }
            _ => panic!("usage must be an error"),
        }
    }

    #[test]
    fn a_line_of_empty_text_is_empty_unless_it_carries_a_mark() {
        assert!(Line::new().is_empty());
        assert!(Line::from("").is_empty());
        assert!(!Line::from(Span::marked(Mark::Up, "")).is_empty());
    }

    #[test]
    fn table_rows_accept_anything_that_becomes_a_line() {
        let mut table = Table::new(&["A", "B"]);
        table.row([Line::from("x"), Span::entity("y").into()]);
        assert_eq!(table.len(), 1);
        assert_eq!(table.rows[0][1], Line::from(Span::entity("y")));
    }
}
