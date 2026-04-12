use crate::diff::types::{DiffKind, DiffNode, FileDiff};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, file_diffs: &[FileDiff], scroll: usize, changed_only: bool) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    let stats = count_stats(file_diffs);
    let header_spans = vec![
        Span::styled("spec-diff", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(format!("+{}", stats.added), Style::default().fg(Color::Green)),
        Span::raw(" "),
        Span::styled(format!("-{}", stats.removed), Style::default().fg(Color::Red)),
        if stats.renamed > 0 {
            Span::styled(format!(" ~>{}", stats.renamed), Style::default().fg(Color::Yellow))
        } else {
            Span::raw("")
        },
    ];
    let header = Paragraph::new(Line::from(header_spans))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    let mut lines = Vec::new();
    for file_diff in file_diffs {
        let file_has_changes = file_diff.nodes.iter().any(has_changes);
        if changed_only && !file_has_changes {
            continue;
        }

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
        Span::raw(" scroll"),
    ]);
    let footer = Paragraph::new(help);
    frame.render_widget(footer, chunks[2]);
}

fn collect_lines(node: &DiffNode, lines: &mut Vec<Line<'_>>, depth: usize, changed_only: bool) {
    if changed_only && node.kind == DiffKind::Unchanged && !has_changes(node) {
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

    let text = match node.kind {
        DiffKind::Renamed => {
            if let Some(old) = &node.old_name {
                format!("{prefix} {indent}{old} -> {}", node.name)
            } else {
                format!("{prefix} {indent}{}", node.name)
            }
        }
        _ => format!("{prefix} {indent}{}", node.name),
    };

    lines.push(Line::from(Span::styled(text, style)));

    for child in &node.children {
        collect_lines(child, lines, depth + 1, changed_only);
    }
}

fn has_changes(node: &DiffNode) -> bool {
    if node.kind != DiffKind::Unchanged {
        return true;
    }
    node.children.iter().any(has_changes)
}

struct Stats {
    added: usize,
    removed: usize,
    renamed: usize,
}

fn count_stats(file_diffs: &[FileDiff]) -> Stats {
    let mut stats = Stats { added: 0, removed: 0, renamed: 0 };
    for fd in file_diffs {
        count_leaf_nodes(&fd.nodes, &mut stats);
    }
    stats
}

fn count_leaf_nodes(nodes: &[DiffNode], stats: &mut Stats) {
    for node in nodes {
        let is_leaf = node.children.is_empty();
        match node.kind {
            DiffKind::Added if is_leaf => stats.added += 1,
            DiffKind::Removed if is_leaf => stats.removed += 1,
            DiffKind::Renamed => stats.renamed += 1,
            _ => {}
        }
        count_leaf_nodes(&node.children, stats);
    }
}
