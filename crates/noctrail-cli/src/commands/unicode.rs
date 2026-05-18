use noctrail_term::{
    Cursor, LineEnding, Position, Selection, SelectionMode, TerminalSnapshot, TerminalState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnicodeCapability {
    Input,
    Selection,
    Copy,
    Cursor,
}

impl UnicodeCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Selection => "selection",
            Self::Copy => "copy",
            Self::Cursor => "cursor",
        }
    }
}

type UnicodeProbe = fn() -> ObservedUnicodeCapabilities;

#[derive(Debug, Clone)]
struct UnicodeTargetSpec {
    name: &'static str,
    probe: UnicodeProbe,
    required: &'static [UnicodeCapability],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ObservedUnicodeCapabilities {
    input: bool,
    selection: bool,
    copy: bool,
    cursor: bool,
}

impl ObservedUnicodeCapabilities {
    fn contains(self, capability: UnicodeCapability) -> bool {
        match capability {
            UnicodeCapability::Input => self.input,
            UnicodeCapability::Selection => self.selection,
            UnicodeCapability::Copy => self.copy,
            UnicodeCapability::Cursor => self.cursor,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [
            UnicodeCapability::Input,
            UnicodeCapability::Selection,
            UnicodeCapability::Copy,
            UnicodeCapability::Cursor,
        ] {
            if self.contains(capability) {
                labels.push(capability.label());
            }
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnicodeProbeReport {
    target: &'static str,
    observed: ObservedUnicodeCapabilities,
    required: Vec<&'static str>,
}

pub(crate) fn run_unicode_matrix(filters: &[String]) -> Result<(), String> {
    let specs = unicode_target_specs();
    let selected = select_unicode_targets(&specs, filters)?;

    for spec in selected {
        let report = run_unicode_target(spec)?;
        println!("{}", format_unicode_report(&report));
    }

    println!("unicode matrix ok");
    Ok(())
}

fn run_unicode_target(spec: &UnicodeTargetSpec) -> Result<UnicodeProbeReport, String> {
    let observed = (spec.probe)();
    let missing = spec
        .required
        .iter()
        .filter(|capability| !observed.contains(**capability))
        .map(|capability| capability.label())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "missing capabilities [{}] from {}",
            missing.join(", "),
            spec.name
        ));
    }

    Ok(UnicodeProbeReport {
        target: spec.name,
        observed,
        required: spec.required.iter().map(|cap| cap.label()).collect(),
    })
}

fn format_unicode_report(report: &UnicodeProbeReport) -> String {
    let required = report.required.join(",");
    let observed = report.observed.labels().join(",");
    format!(
        "pass {} required={} observed={}",
        report.target, required, observed
    )
}

fn select_unicode_targets<'a>(
    specs: &'a [UnicodeTargetSpec],
    filters: &[String],
) -> Result<Vec<&'a UnicodeTargetSpec>, String> {
    if filters.is_empty() {
        return Ok(specs.iter().collect());
    }

    let mut selected = Vec::new();
    for filter in filters {
        let Some(spec) = specs.iter().find(|spec| spec.name == filter) else {
            return Err(format!("unknown unicode target: {filter}"));
        };
        if !selected
            .iter()
            .any(|existing: &&UnicodeTargetSpec| existing.name == spec.name)
        {
            selected.push(spec);
        }
    }

    Ok(selected)
}

fn unicode_target_specs() -> Vec<UnicodeTargetSpec> {
    vec![
        UnicodeTargetSpec {
            name: "cjk",
            probe: probe_cjk_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
        UnicodeTargetSpec {
            name: "emoji",
            probe: probe_emoji_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
        UnicodeTargetSpec {
            name: "combining",
            probe: probe_combining_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
        UnicodeTargetSpec {
            name: "fullwidth",
            probe: probe_fullwidth_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
    ]
}

fn probe_cjk_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_str("中a");
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 2 },
        },
        "中a",
        "中a",
        Cursor { row: 0, col: 3 },
        |snapshot| {
            let Some(row) = snapshot.rows.first() else {
                return false;
            };
            row.cells.first().is_some_and(|cell| cell.text == "中")
                && row.cells.get(1).is_some_and(|cell| cell.wide_continuation)
        },
    )
}

fn probe_emoji_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_str("🙂x");
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 2 },
        },
        "🙂x",
        "🙂x",
        Cursor { row: 0, col: 3 },
        |snapshot| {
            let Some(row) = snapshot.rows.first() else {
                return false;
            };
            row.cells.first().is_some_and(|cell| cell.text == "🙂")
                && row.cells.get(1).is_some_and(|cell| cell.wide_continuation)
        },
    )
}

fn probe_combining_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_char('e');
    terminal.advance_char('\u{301}');
    terminal.advance_char('x');
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 1 },
        },
        "e\u{301}x",
        "e\u{301}x",
        Cursor { row: 0, col: 2 },
        |snapshot| {
            snapshot.rows.first().is_some_and(|row| {
                row.cells
                    .first()
                    .is_some_and(|cell| cell.text == "e\u{301}")
            })
        },
    )
}

fn probe_fullwidth_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_str("Ａb");
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 2 },
        },
        "Ａb",
        "Ａb",
        Cursor { row: 0, col: 3 },
        |snapshot| {
            let Some(row) = snapshot.rows.first() else {
                return false;
            };
            row.cells.first().is_some_and(|cell| cell.text == "Ａ")
                && row.cells.get(1).is_some_and(|cell| cell.wide_continuation)
        },
    )
}

fn verify_unicode_case(
    terminal: &mut TerminalState,
    selection: Selection,
    expected_render: &str,
    expected_copy: &str,
    expected_cursor: Cursor,
    input_matches: impl Fn(&TerminalSnapshot) -> bool,
) -> ObservedUnicodeCapabilities {
    let snapshot = terminal.snapshot();
    let input = snapshot
        .rows
        .first()
        .is_some_and(|row| row.rendered_text().starts_with(expected_render))
        && input_matches(&snapshot);
    let cursor = snapshot.cursor == expected_cursor;

    let normalized = selection.clone().normalized();
    terminal.set_selection(Some(selection));
    let selection_snapshot = terminal.snapshot();
    let selection_seen = selection_snapshot.selection.as_ref() == Some(&normalized);
    let copy = terminal.selection_text(LineEnding::Lf).as_deref() == Some(expected_copy);

    ObservedUnicodeCapabilities {
        input,
        selection: selection_seen,
        copy,
        cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_target_filter_accepts_named_targets() {
        let specs = unicode_target_specs();
        let selected =
            select_unicode_targets(&specs, &[String::from("cjk"), String::from("fullwidth")])
                .expect("filters should resolve");

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, "cjk");
        assert_eq!(selected[1].name, "fullwidth");
    }

    #[test]
    fn unicode_matrix_builtin_probes_pass() {
        run_unicode_matrix(&[]).expect("builtin unicode probes should pass");
    }
}
