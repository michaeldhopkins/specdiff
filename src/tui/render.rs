use crate::diff::types::{DiffKind, DiffNode, FileDiff, Stats};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub struct RenderResult {
    pub section_offsets: Vec<usize>,
    pub max_scroll: usize,
}

pub fn render(frame: &mut Frame, file_diffs: &[FileDiff], scroll: usize, changed_only: bool) -> RenderResult {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    let stats = Stats::from_file_diffs(file_diffs);
    let header_spans = if stats.is_empty() {
        vec![
            Span::styled("specdiff", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("  No test outline changes", Style::default().fg(Color::DarkGray)),
        ]
    } else {
        vec![
            Span::styled("specdiff", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("+{}", stats.added), Style::default().fg(Color::Green)),
            Span::raw(" "),
            Span::styled(format!("-{}", stats.removed), Style::default().fg(Color::Red)),
            if stats.renamed > 0 {
                Span::styled(format!(" ~>{}", stats.renamed), Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            },
        ]
    };
    let header = Paragraph::new(Line::from(header_spans))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    let mut lines = Vec::new();
    let mut section_offsets = Vec::new();
    for file_diff in file_diffs {
        let file_has_changes = file_diff.nodes.iter().any(DiffNode::has_changes);
        if changed_only && !file_has_changes {
            continue;
        }

        section_offsets.push(lines.len());
        lines.push(Line::from(Span::styled(
            format!("  {}", file_diff.path),
            Style::default().add_modifier(Modifier::BOLD),
        )));

        for node in &file_diff.nodes {
            collect_lines(node, &mut lines, 1, changed_only);
        }

        lines.push(Line::from(""));
    }

    let max_scroll = lines.len().saturating_sub(chunks[1].height as usize);
    let effective_scroll = scroll.min(max_scroll);

    let body = Paragraph::new(lines).scroll((effective_scroll as u16, 0));
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

fn collect_lines(node: &DiffNode, lines: &mut Vec<Line<'_>>, depth: usize, changed_only: bool) {
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

    lines.push(Line::from(Span::styled(text, style)));

    for child in &node.children {
        collect_lines(child, lines, depth + 1, changed_only);
    }
}

