use std::io::IsTerminal;

use console::{measure_text_width, truncate_str};

use super::output::{
    Block, CommandOutput, Failure, Fields, Hint, Line, Node, OutputLine, Span, Table,
};
use crate::terminal::{self, Tone};

const COLUMN_GAP: usize = 3;
const MIN_COLUMN: usize = 8;

struct Glyphs {
    title: &'static str,
    rail: &'static str,
    ok: &'static str,
    error: &'static str,
    warn: &'static str,
    note: &'static str,
    dot: &'static str,
    ellipsis: &'static str,
    usage: &'static str,
}

const FANCY: Glyphs = Glyphs {
    title: "◆",
    rail: "│",
    ok: "✓",
    error: "✗",
    warn: "!",
    note: "–",
    dot: "·",
    ellipsis: "…",
    usage: "usage ",
};

const PLAIN: Glyphs = Glyphs {
    title: "#",
    rail: "|",
    ok: "ok:",
    error: "error:",
    warn: "warn:",
    note: "-",
    dot: "-",
    ellipsis: "...",
    usage: "usage:",
};

pub struct Renderer {
    colored: bool,
    width: Option<usize>,
}

impl Renderer {
    pub fn new(colored: bool, width: Option<usize>) -> Self {
        Self { colored, width }
    }

    pub fn detect() -> Self {
        let width = std::io::stdout()
            .is_terminal()
            .then(|| console::Term::stdout().size_checked())
            .flatten()
            .map(|(_, columns)| usize::from(columns));
        Self::new(terminal::color::enabled(), width)
    }

    pub fn render(&self, output: &CommandOutput) -> String {
        let glyphs = self.glyphs();
        let lines = match output {
            CommandOutput::Block(block) => self.block(block),
            CommandOutput::Table { table, footer } => {
                let mut lines = vec![table.to_string()];
                lines.extend(footer.iter().map(|footer| footer.trim().to_string()));
                lines
            }
            CommandOutput::Lines(lines) => {
                lines.iter().map(|line| self.output_line(line)).collect()
            }
            CommandOutput::Success(message) => vec![self.status(Tone::Ok, glyphs.ok, message)],
            CommandOutput::Note(message) => {
                vec![self.paint(Tone::Muted, &format!("{} {message}", glyphs.note))]
            }
            CommandOutput::Error(failure) => self.failure(failure),
            CommandOutput::None => Vec::new(),
        };
        lines.join("\n")
    }

    fn glyphs(&self) -> &'static Glyphs {
        if self.colored { &FANCY } else { &PLAIN }
    }

    fn paint(&self, tone: Tone, text: &str) -> String {
        tone.paint(text, self.colored).to_string()
    }

    fn status(&self, tone: Tone, glyph: &str, message: &str) -> String {
        format!("{} {message}", self.paint(tone, glyph))
    }

    fn output_line(&self, line: &OutputLine) -> String {
        let glyphs = self.glyphs();
        match line {
            OutputLine::Info(message) => message.clone(),
            OutputLine::Success(message) => self.status(Tone::Ok, glyphs.ok, message),
            OutputLine::Warning(message) => self.status(Tone::Warn, glyphs.warn, message),
            OutputLine::Error(message) => self.status(Tone::Err, glyphs.error, message),
        }
    }

    fn failure(&self, failure: &Failure) -> Vec<String> {
        let glyphs = self.glyphs();
        let mut lines = vec![self.status(Tone::Err, glyphs.error, &failure.message)];
        match &failure.hint {
            Some(Hint::Usage(usage)) => lines.push(format!(
                "  {} {}",
                self.paint(Tone::Muted, glyphs.usage),
                self.line(&usage_line(usage))
            )),
            Some(Hint::Note(note)) => lines.push(format!("  {}", self.paint(Tone::Muted, note))),
            None => {}
        }
        lines
    }

    fn block(&self, block: &Block) -> Vec<String> {
        let glyphs = self.glyphs();
        let mut head = self.paint(Tone::Title, &format!("{} {}", glyphs.title, block.title));
        if !block.meta.is_empty() {
            head.push(' ');
            head.push_str(&self.paint(Tone::Muted, glyphs.dot));
            head.push(' ');
            head.push_str(&self.line(&muted(&block.meta)));
        }

        let rail = self.paint(Tone::Brand, glyphs.rail);
        let budget = self
            .width
            .map(|width| width.saturating_sub(measure_text_width(glyphs.rail) + 1));

        let mut lines = vec![head];
        for node in &block.body {
            for body in self.node(node, budget) {
                if body.is_empty() {
                    lines.push(rail.clone());
                } else {
                    lines.push(format!("{rail} {body}"));
                }
            }
        }
        lines
    }

    fn node(&self, node: &Node, budget: Option<usize>) -> Vec<String> {
        match node {
            Node::Gap => vec![String::new()],
            Node::Heading(text) => vec![self.paint(Tone::Heading, &text.to_uppercase())],
            Node::Line(line) => vec![self.line(line)],
            Node::Fields(fields) => self.fields(fields),
            Node::Table(table) => self.table(table, budget),
        }
    }

    fn fields(&self, fields: &Fields) -> Vec<String> {
        let label_width = fields
            .0
            .iter()
            .map(|(label, _)| measure_text_width(label))
            .max()
            .unwrap_or(0);
        fields
            .0
            .iter()
            .map(|(label, value)| {
                let padding = " ".repeat(label_width - measure_text_width(label) + 2);
                format!(
                    "{}{padding}{}",
                    self.paint(Tone::Muted, label),
                    self.line(value)
                )
                .trim_end()
                .to_string()
            })
            .collect()
    }

    fn table(&self, table: &Table, budget: Option<usize>) -> Vec<String> {
        let columns = table
            .rows
            .iter()
            .map(Vec::len)
            .chain(std::iter::once(table.headers.len()))
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return Vec::new();
        }

        let mut widths = vec![0; columns];
        for (index, header) in table.headers.iter().enumerate() {
            widths[index] = measure_text_width(header);
        }
        for row in &table.rows {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(self.line_width(cell));
            }
        }

        if let Some(budget) = budget {
            let total = widths.iter().sum::<usize>() + COLUMN_GAP * (columns - 1);
            if total > budget {
                let last = columns - 1;
                let others = total - widths[last];
                widths[last] = budget
                    .saturating_sub(others)
                    .max(MIN_COLUMN)
                    .min(widths[last]);
            }
        }

        let mut lines = Vec::with_capacity(table.rows.len() + 1);
        if !table.headers.is_empty() {
            let header: Vec<Line> = table
                .headers
                .iter()
                .map(|header| Span::muted(header.to_uppercase()).into())
                .collect();
            lines.push(self.row(&header, &widths));
        }
        for row in &table.rows {
            lines.push(self.row(row, &widths));
        }
        lines
    }

    fn row(&self, cells: &[Line], widths: &[usize]) -> String {
        let empty = Line::new();
        let last = widths.len() - 1;
        let mut out = String::new();
        for (index, width) in widths.iter().enumerate() {
            let cell = self.truncate(cells.get(index).unwrap_or(&empty), *width);
            out.push_str(&self.line(&cell));
            if index < last {
                let padding = width - self.line_width(&cell) + COLUMN_GAP;
                out.push_str(&" ".repeat(padding));
            }
        }
        out.trim_end().to_string()
    }

    fn span_text(&self, span: &Span) -> String {
        match span.mark {
            Some(mark) if self.colored && span.text.is_empty() => mark.glyph().to_string(),
            Some(mark) if self.colored => format!("{} {}", mark.glyph(), span.text),
            _ => span.text.clone(),
        }
    }

    fn line_width(&self, line: &Line) -> usize {
        line.0
            .iter()
            .map(|span| measure_text_width(&self.span_text(span)))
            .sum()
    }

    fn line(&self, line: &Line) -> String {
        line.0
            .iter()
            .map(|span| self.paint(span.tone, &self.span_text(span)))
            .collect()
    }

    fn truncate(&self, line: &Line, width: usize) -> Line {
        if self.line_width(line) <= width {
            return line.clone();
        }
        let ellipsis = self.glyphs().ellipsis;
        let mut room = width.saturating_sub(measure_text_width(ellipsis));
        let mut spans = Vec::new();
        for span in &line.0 {
            let text = self.span_text(span);
            let span_width = measure_text_width(&text);
            if span_width <= room {
                room -= span_width;
                spans.push(span.clone());
                continue;
            }
            let cut = truncate_str(&text, room, "");
            spans.push(Span::new(format!("{cut}{ellipsis}"), span.tone));
            break;
        }
        Line(spans)
    }
}

fn muted(line: &Line) -> Line {
    Line(
        line.0
            .iter()
            .map(|span| match span.tone {
                Tone::Plain => Span {
                    tone: Tone::Muted,
                    ..span.clone()
                },
                _ => span.clone(),
            })
            .collect(),
    )
}

pub fn usage_line(usage: &str) -> Line {
    let mut spans = Vec::new();
    for (index, token) in usage.split_whitespace().enumerate() {
        if index > 0 {
            spans.push(Span::plain(" "));
        }
        spans.push(match token.chars().next() {
            Some('<') => Span::entity(token),
            Some('[') => Span::muted(token),
            _ => Span::plain(token),
        });
    }
    Line(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::console::output::Fields;
    use crate::terminal::Mark;

    fn plain(output: &CommandOutput) -> String {
        Renderer::new(false, None).render(output)
    }

    fn fancy(output: &CommandOutput, width: Option<usize>) -> String {
        console::strip_ansi_codes(&Renderer::new(true, width).render(output)).into_owned()
    }

    fn players() -> CommandOutput {
        let mut table = Table::new(&["Player", "Server", "Address"]);
        table
            .row([
                Span::entity("Notch").into(),
                Line::from("lobby"),
                Span::muted("203.0.113.4").into(),
            ])
            .row([
                Span::entity("Dinnerbone").into(),
                Line::from("survival"),
                Span::muted("192.0.2.33").into(),
            ]);
        Block::new("Players").meta("2 online").table(table).into()
    }

    #[test]
    fn a_block_without_color_uses_ascii_markers() {
        assert_eq!(
            plain(&players()),
            "# Players - 2 online\n\
             | PLAYER       SERVER     ADDRESS\n\
             | Notch        lobby      203.0.113.4\n\
             | Dinnerbone   survival   192.0.2.33"
        );
    }

    #[test]
    fn a_block_with_color_draws_the_diamond_and_the_rail() {
        assert_eq!(
            fancy(&players(), None),
            "◆ Players · 2 online\n\
             │ PLAYER       SERVER     ADDRESS\n\
             │ Notch        lobby      203.0.113.4\n\
             │ Dinnerbone   survival   192.0.2.33"
        );
    }

    #[test]
    fn the_last_column_is_cut_to_the_terminal_width() {
        let rendered = fancy(&players(), Some(36));
        for line in rendered.lines() {
            assert!(measure_text_width(line) <= 36, "{line:?}");
        }
        assert!(
            rendered.contains("│ Notch        lobby      203.0.113…"),
            "{rendered}"
        );
    }

    #[test]
    fn marks_show_only_with_color() {
        let mut table = Table::bare();
        table.row([Line::from("lobby"), Span::marked(Mark::Up, "online").into()]);
        let output: CommandOutput = Block::new("Servers").table(table).into();
        assert_eq!(fancy(&output, None), "◆ Servers\n│ lobby   ● online");
        assert_eq!(plain(&output), "# Servers\n| lobby   online");
    }

    #[test]
    fn fields_align_their_values() {
        let output: CommandOutput = Block::new("survival")
            .fields(
                Fields::new()
                    .field("mode", "client_only")
                    .field("players", "8 / 100"),
            )
            .gap()
            .heading("more")
            .into();
        assert_eq!(
            plain(&output),
            "# survival\n| mode     client_only\n| players  8 / 100\n|\n| MORE"
        );
    }

    #[test]
    fn a_usage_error_shows_the_usage_under_the_message() {
        let output = CommandOutput::usage("kick <player> [reason...]");
        assert_eq!(
            plain(&output),
            "error: Missing arguments\n  usage: kick <player> [reason...]"
        );
        assert_eq!(
            fancy(&output, None),
            "✗ Missing arguments\n  usage  kick <player> [reason...]"
        );
    }

    #[test]
    fn single_line_results_carry_their_marker() {
        assert_eq!(plain(&CommandOutput::Success("done".into())), "ok: done");
        assert_eq!(
            fancy(&CommandOutput::Success("done".into()), None),
            "✓ done"
        );
        assert_eq!(plain(&CommandOutput::Note("nobody".into())), "- nobody");
        assert_eq!(
            plain(&CommandOutput::Lines(vec![
                OutputLine::Success("a".into()),
                OutputLine::Warning("b".into()),
                OutputLine::Error("c".into()),
                OutputLine::Info("d".into()),
            ])),
            "ok: a\nwarn: b\nerror: c\nd"
        );
        assert_eq!(plain(&CommandOutput::None), "");
    }

    #[test]
    fn usage_tokens_are_toned_by_kind() {
        let line = usage_line("ban <player> [duration]");
        let tones: Vec<Tone> = line.0.iter().map(|span| span.tone).collect();
        assert_eq!(
            tones,
            [
                Tone::Plain,
                Tone::Plain,
                Tone::Entity,
                Tone::Plain,
                Tone::Muted
            ]
        );
    }
}
