use crate::diff::types::{DiffKind, DiffNode, FileDiff, Stats};
use crate::output::truncate::{truncate_unchanged_runs, CONTEXT_HEAD, CONTEXT_TAIL};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub struct RenderResult {
    pub section_offsets: Vec<usize>,
    pub max_scroll: usize,
}

#[derive(Clone, Copy, Default)]
pub struct RenderOptions {
    pub changed_only: bool,
    pub full_context: bool,
}

#[derive(Clone, Copy)]
enum LineKind {
    Spec(DiffKind),
    Other,
    Ellipsis,
}

struct TuiLine {
    kind: LineKind,
    line: Line<'static>,
    is_section_start: bool,
}

pub fn render(
    frame: &mut Frame,
    file_diffs: &[FileDiff],
    scroll: usize,
    opts: RenderOptions,
) -> RenderResult {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    let header = Paragraph::new(Line::from(header_spans(file_diffs)))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    let tui_lines = build_tui_lines(file_diffs, opts);
    let mut section_offsets: Vec<usize> = Vec::new();
    let mut body_lines: Vec<Line<'static>> = Vec::with_capacity(tui_lines.len());
    for (idx, item) in tui_lines.into_iter().enumerate() {
        if item.is_section_start {
            section_offsets.push(idx);
        }
        body_lines.push(item.line);
    }

    let max_scroll = body_lines.len().saturating_sub(chunks[1].height as usize);
    let effective_scroll = scroll.min(max_scroll);

    let body = Paragraph::new(body_lines).scroll((effective_scroll as u16, 0));
    frame.render_widget(body, chunks[1]);

    let help = Line::from(vec![
        Span::styled("[q]", Style::default().fg(Color::DarkGray)),
        Span::raw("uit  "),
        Span::styled("[c]", Style::default().fg(Color::DarkGray)),
        Span::raw("hanged-only  "),
        Span::styled("[j/k]", Style::default().fg(Color::DarkGray)),
        Span::raw(" next/prev file"),
    ]);
    let footer = Paragraph::new(help);
    frame.render_widget(footer, chunks[2]);

    RenderResult { section_offsets, max_scroll }
}

fn header_spans(file_diffs: &[FileDiff]) -> Vec<Span<'static>> {
    let stats = Stats::from_file_diffs(file_diffs);
    if stats.is_empty() {
        return vec![
            Span::styled("specdiff", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("  No test outline changes", Style::default().fg(Color::DarkGray)),
        ];
    }
    let mut spans = vec![
        Span::styled("specdiff", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(format!("+{}", stats.added), Style::default().fg(Color::Green)),
        Span::raw(" "),
        Span::styled(format!("-{}", stats.removed), Style::default().fg(Color::Red)),
    ];
    if stats.renamed > 0 {
        spans.push(Span::styled(format!(" ~>{}", stats.renamed), Style::default().fg(Color::Yellow)));
    }
    spans
}

fn build_tui_lines(file_diffs: &[FileDiff], opts: RenderOptions) -> Vec<TuiLine> {
    let mut lines: Vec<TuiLine> = Vec::new();
    for file_diff in file_diffs {
        let file_has_changes = file_diff.nodes.iter().any(DiffNode::has_changes);
        if opts.changed_only && !file_has_changes {
            continue;
        }
        lines.push(TuiLine {
            kind: LineKind::Other,
            line: Line::from(Span::styled(
                format!("  {}", file_diff.path),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            is_section_start: true,
        });
        for node in &file_diff.nodes {
            collect_lines(node, &mut lines, 1, opts.changed_only);
        }
        lines.push(TuiLine {
            kind: LineKind::Other,
            line: Line::from(""),
            is_section_start: false,
        });
    }

    if !opts.changed_only && !opts.full_context {
        truncate_unchanged_runs(
            &mut lines,
            CONTEXT_HEAD,
            CONTEXT_TAIL,
            |item| matches!(item.kind, LineKind::Spec(DiffKind::Unchanged)),
            make_ellipsis_tui_line,
        );
    }

    lines
}

fn make_ellipsis_tui_line(hidden: usize) -> TuiLine {
    let plural = if hidden == 1 { "" } else { "s" };
    TuiLine {
        kind: LineKind::Ellipsis,
        line: Line::from(Span::styled(
            format!("    ... ({hidden} hidden line{plural})"),
            Style::default().fg(Color::DarkGray),
        )),
        is_section_start: false,
    }
}

fn collect_lines(node: &DiffNode, lines: &mut Vec<TuiLine>, depth: usize, changed_only: bool) {
    if changed_only && node.kind == DiffKind::Unchanged && !node.has_changes() {
        return;
    }

    let indent = "  ".repeat(depth);
    let (prefix, style) = match node.kind {
        DiffKind::Added => ("+ ", Style::default().fg(Color::Green)),
        DiffKind::Removed => ("- ", Style::default().fg(Color::Red)),
        DiffKind::Renamed => ("->", Style::default().fg(Color::Yellow)),
        DiffKind::Modified => ("~ ", Style::default().fg(Color::Cyan)),
        DiffKind::Unchanged => ("  ", Style::default().fg(Color::DarkGray)),
    };

    let suffix = node
        .param_cases
        .map(|n| format!(" [{n} cases]"))
        .unwrap_or_default();

    let text = match node.kind {
        DiffKind::Renamed => {
            if let Some(old) = &node.old_name {
                format!("{prefix} {indent}{old} -> {}{suffix}", node.name)
            } else {
                format!("{prefix} {indent}{}{suffix}", node.name)
            }
        }
        _ => format!("{prefix} {indent}{}{suffix}", node.name),
    };

    lines.push(TuiLine {
        kind: LineKind::Spec(node.kind),
        line: Line::from(Span::styled(text, style)),
        is_section_start: false,
    });

    for child in &node.children {
        collect_lines(child, lines, depth + 1, changed_only);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_truncating() -> RenderOptions {
        RenderOptions { changed_only: false, full_context: false }
    }

    fn unchanged(name: &str) -> DiffNode {
        DiffNode {
            name: name.into(),
            kind: DiffKind::Unchanged,
            old_name: None,
            param_cases: None,
            old_param_cases: None,
            children: vec![],
        }
    }

    fn added(name: &str) -> DiffNode {
        DiffNode {
            name: name.into(),
            kind: DiffKind::Added,
            old_name: None,
            param_cases: None,
            old_param_cases: None,
            children: vec![],
        }
    }

    fn diffs_with_long_run() -> Vec<FileDiff> {
        let mut children: Vec<DiffNode> = (0..10).map(|i| unchanged(&format!("ctx {i}"))).collect();
        children.push(added("new spec"));
        vec![FileDiff {
            path: "m::u".into(),
            nodes: vec![DiffNode {
                name: "group".into(),
                kind: DiffKind::Modified,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children,
            }],
        }]
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn tui_truncates_long_unchanged_run() {
        let lines = build_tui_lines(&diffs_with_long_run(), opts_truncating());
        let rendered: Vec<String> = lines.iter().map(|l| line_text(&l.line)).collect();
        let joined = rendered.join("\n");
        assert!(joined.contains("ctx 0"));
        assert!(joined.contains("ctx 1"));
        assert!(joined.contains("ctx 2"));
        assert!(!joined.contains("ctx 4"), "middle of run is dropped");
        assert!(joined.contains("ctx 8"));
        assert!(joined.contains("ctx 9"));
        assert!(joined.contains("... (5 hidden lines)"));
        assert!(joined.contains("new spec"));
    }

    #[test]
    fn tui_full_context_preserves_all_lines() {
        let opts = RenderOptions { changed_only: false, full_context: true };
        let lines = build_tui_lines(&diffs_with_long_run(), opts);
        let joined: String = lines.iter().map(|l| line_text(&l.line)).collect::<Vec<_>>().join("\n");
        for i in 0..10 {
            assert!(joined.contains(&format!("ctx {i}")), "full-context keeps ctx {i}");
        }
        assert!(!joined.contains("hidden line"));
    }

    #[test]
    fn tui_changed_only_drops_unchanged_no_ellipsis() {
        let opts = RenderOptions { changed_only: true, full_context: false };
        let lines = build_tui_lines(&diffs_with_long_run(), opts);
        let joined: String = lines.iter().map(|l| line_text(&l.line)).collect::<Vec<_>>().join("\n");
        for i in 0..10 {
            assert!(!joined.contains(&format!("ctx {i}")));
        }
        assert!(!joined.contains("hidden line"));
        assert!(joined.contains("new spec"));
    }

    #[test]
    fn tui_file_headers_marked_as_section_starts() {
        let diffs = vec![
            FileDiff {
                path: "file_a".into(),
                nodes: vec![added("spec_a")],
            },
            FileDiff {
                path: "file_b".into(),
                nodes: vec![added("spec_b")],
            },
        ];
        let lines = build_tui_lines(&diffs, opts_truncating());
        let starts: Vec<String> = lines
            .iter()
            .filter(|l| l.is_section_start)
            .map(|l| line_text(&l.line))
            .collect();
        assert_eq!(starts.len(), 2);
        assert!(starts[0].contains("file_a"));
        assert!(starts[1].contains("file_b"));
    }
}
