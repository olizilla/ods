use crate::commands::info::model::{InfoRecord, InfoRelationship, InfoRole};
use comfy_table::presets::{ASCII_MARKDOWN, UTF8_FULL_CONDENSED};
use comfy_table::{Cell, ColumnConstraint, ContentArrangement, Table, Width};

pub use crate::ansi::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyle {
    Table,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsiveBand {
    Narrow, // 58
    Medium, // 78
    Wide,   // 98
}

impl ResponsiveBand {
    pub fn width(&self) -> u16 {
        match self {
            ResponsiveBand::Narrow => 58,
            ResponsiveBand::Medium => 78,
            ResponsiveBand::Wide => 98,
        }
    }

    pub fn from_width(w: u16) -> Self {
        if w >= 98 {
            ResponsiveBand::Wide
        } else if w >= 78 {
            ResponsiveBand::Medium
        } else {
            ResponsiveBand::Narrow
        }
    }

    pub fn has_start_date(&self) -> bool {
        matches!(self, ResponsiveBand::Medium | ResponsiveBand::Wide)
    }

    pub fn has_end_date(&self) -> bool {
        matches!(self, ResponsiveBand::Wide)
    }
}

pub struct RenderOptions<'a> {
    pub band: ResponsiveBand,
    pub all: bool,
    pub color: bool,
    pub source_header: &'a [String],
    pub style: TableStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyType {
    StillOpenOnClosed,
    StartsAfterClosed,
    EndsAfterClosed,
}

impl AnomalyType {
    pub fn message(&self, count: usize) -> String {
        let plural = if count == 1 { "row" } else { "rows" };
        match self {
            AnomalyType::StillOpenOnClosed => {
                format!("! {} {} still open on a closed organisation", count, plural)
            }
            AnomalyType::StartsAfterClosed => {
                format!("! {} {} starts after the organisation closed", count, plural)
            }
            AnomalyType::EndsAfterClosed => {
                format!("! {} {} ends after the organisation closed", count, plural)
            }
        }
    }
}

/// Checks a single item for anomalies relative to the parent record's operational end date.
pub fn check_anomaly(
    record_end: Option<&str>,
    item_start: Option<&str>,
    item_end: Option<&str>,
) -> Option<AnomalyType> {
    let rec_end = record_end?;

    // 1. Record is closed and the row has no end date
    if item_end.is_none() {
        return Some(AnomalyType::StillOpenOnClosed);
    }

    // 2. Row start > record end
    if let Some(st) = item_start {
        if st > rec_end {
            return Some(AnomalyType::StartsAfterClosed);
        }
    }

    // 3. Row end > record end
    if let Some(en) = item_end {
        if en > rec_end {
            return Some(AnomalyType::EndsAfterClosed);
        }
    }

    None
}

/// Canonical order for Relationship Types
const CANONICAL_RE_ORDER: &[(&str, &str)] = &[
    ("RE2", "sub-division of"),
    ("RE3", "directed by"),
    ("RE4", "commissioned by"),
    ("RE5", "in the geography of"),
    ("RE6", "operated by"),
    ("RE8", "partner to"),
    ("RE9", "nominated payee for"),
    ("RE10", "covid nominated payee for"),
    ("RE11", "constituent of"),
];

pub fn canonical_re_rank_and_phrase(rel_code: &str) -> (usize, &'static str) {
    for (idx, &(code, phrase)) in CANONICAL_RE_ORDER.iter().enumerate() {
        if code.eq_ignore_ascii_case(rel_code) {
            return (idx, phrase);
        }
    }
    (CANONICAL_RE_ORDER.len(), "related to")
}

pub fn parse_ro_number(code: &str) -> u32 {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().unwrap_or(u32::MAX)
}

fn apply_column_widths(table: &mut Table, widths: &[u16]) {
    table.set_content_arrangement(ContentArrangement::Disabled);
    for (i, w) in widths.iter().enumerate() {
        if let Some(col) = table.column_mut(i) {
            col.set_constraint(ColumnConstraint::Absolute(Width::Fixed(w + 2)));
        }
    }
}

fn create_base_table(color: bool, style: TableStyle) -> Table {
    let mut table = Table::new();
    match style {
        TableStyle::Markdown => {
            table.load_style(ASCII_MARKDOWN);
            table.set_content_arrangement(ContentArrangement::Disabled);
        }
        TableStyle::Table => {
            table.load_style(UTF8_FULL_CONDENSED);
            table.set_truncation_indicator("…");
            if !color {
                table.force_no_tty();
            }
        }
    }
    table
}

pub fn render_info(record: &InfoRecord, options: &RenderOptions) -> String {
    let mut out = String::new();

    let is_record_closed = record.operational_end.is_some()
        || record.status.eq_ignore_ascii_case("inactive");

    // 1. Source header
    for line in options.source_header {
        out.push_str(&format!("{line}\n"));
    }

    // 2. Status line
    let status_str = if options.color {
        if is_record_closed {
            format!("{ANSI_RED}inactive{ANSI_RESET}")
        } else {
            format!("{ANSI_GREEN}active{ANSI_RESET}")
        }
    } else {
        (if is_record_closed { "inactive" } else { "active" }).to_string()
    };

    let last_change_str = record.last_changed.as_deref().unwrap_or("—");
    let status_line = format!(
        "* Status: {}, Class: {}, Last Change: {}",
        status_str, record.record_class, last_change_str
    );

    if options.color {
        // Color prefix and suffix with muted, keeping status_str colored
        out.push_str(&format!(
            "{ANSI_MUTED}* Status: {ANSI_RESET}{}{ANSI_MUTED}, Class: {}, Last Change: {}{ANSI_RESET}\n",
            status_str, record.record_class, last_change_str
        ));
    } else {
        out.push_str(&format!("{}\n", status_line));
    }

    if options.style == TableStyle::Markdown {
        out.push('\n');
    }

    // 3. Fields Table
    let fields_table_str = render_fields_table(record, options);
    out.push_str(&fields_table_str);
    out.push('\n');

    // 4. Roles Table
    let role_anomalies = if !record.roles.is_empty() {
        let (roles_table_str, anomalies) =
            render_roles_table(record, options, is_record_closed);
        out.push('\n');
        out.push_str(&roles_table_str);
        out.push('\n');
        anomalies
    } else {
        Vec::new()
    };

    // 5. Relationships Table
    let outbound_count = record
        .relationships
        .iter()
        .filter(|r| r.direction == "outbound")
        .count();

    let rel_anomalies = if outbound_count > 0 {
        let (rels_table_str, anomalies) =
            render_relationships_table(record, options, is_record_closed);
        out.push('\n');
        out.push_str(&rels_table_str);
        out.push('\n');
        anomalies
    } else {
        Vec::new()
    };

    // 6. Footers: Anomalies
    let mut class_counts = [0usize; 3];
    for a in role_anomalies.into_iter().chain(rel_anomalies) {
        match a {
            AnomalyType::StillOpenOnClosed => class_counts[0] += 1,
            AnomalyType::StartsAfterClosed => class_counts[1] += 1,
            AnomalyType::EndsAfterClosed => class_counts[2] += 1,
        }
    }

    let mut first = true;
    for (anom, count) in [
        (AnomalyType::StillOpenOnClosed, class_counts[0]),
        (AnomalyType::StartsAfterClosed, class_counts[1]),
        (AnomalyType::EndsAfterClosed, class_counts[2]),
    ] {
        if count > 0 {
            if first {
                out.push('\n');
                first = false;
            }
            if options.style == TableStyle::Markdown {
                out.push_str(&format!("> {}\n\n", anom.message(count)));
            } else {
                out.push_str(&anom.message(count));
                out.push('\n');
            }
        }
    }

    out
}

fn titlecase_country(country: Option<&str>) -> String {
    match country {
        Some(c) if c.eq_ignore_ascii_case("england") => "England".to_string(),
        Some(c) if c.eq_ignore_ascii_case("scotland") => "Scotland".to_string(),
        Some(c) if c.eq_ignore_ascii_case("wales") => "Wales".to_string(),
        Some(c) if c.eq_ignore_ascii_case("northern ireland") => "Northern Ireland".to_string(),
        Some(c) => c.to_string(),
        None => "—".to_string(),
    }
}

fn render_fields_table(record: &InfoRecord, options: &RenderOptions) -> String {
    let mut table = create_base_table(options.color, options.style);

    let header_code = if options.color {
        format!("{ANSI_BOLD}{ANSI_CYAN}{}{ANSI_RESET}", record.ods_code)
    } else {
        record.ods_code.clone()
    };
    table.set_header(vec![Cell::new("ODS Code"), Cell::new(&header_code)]);

    if options.style == TableStyle::Table {
        let widths = match options.band {
            ResponsiveBand::Narrow => [10, 41],
            ResponsiveBand::Medium => [12, 59],
            ResponsiveBand::Wide => [12, 79],
        };
        apply_column_widths(&mut table, &widths);
    }

    // Name (bold if color)
    let name_val = if options.color {
        format!("{ANSI_BOLD}{}{ANSI_RESET}", record.name)
    } else {
        record.name.clone()
    };
    table.add_row(vec![Cell::new("Name"), Cell::new(&name_val)]);

    // Address
    let addr_val = record.address.as_deref().unwrap_or("—");
    table.add_row(vec![Cell::new("Address"), Cell::new(addr_val)]);

    // Country (titlecased)
    let country_val = titlecase_country(record.country.as_deref());
    table.add_row(vec![Cell::new("Country"), Cell::new(&country_val)]);

    // UPRN
    let uprn_val = record.uprn.as_deref().unwrap_or("—");
    table.add_row(vec![Cell::new("UPRN"), Cell::new(uprn_val)]);

    // Website
    let web_val = record.website.as_deref().unwrap_or("—");
    table.add_row(vec![Cell::new("Website"), Cell::new(web_val)]);

    // Telephone
    let tel_val = record.telephone.as_deref().unwrap_or("—");
    table.add_row(vec![Cell::new("Telephone"), Cell::new(tel_val)]);

    // Opened
    let opened_val = record.operational_start.as_deref().unwrap_or("—");
    table.add_row(vec![Cell::new("Opened"), Cell::new(opened_val)]);

    // Closed (if present)
    if let Some(ref closed) = record.operational_end {
        table.add_row(vec![Cell::new("Closed"), Cell::new(closed)]);
    }

    // Succeeded by (one row per live successor)
    for (i, succ) in record.successors_live.iter().enumerate() {
        let label = if i == 0 { "Succeeded by" } else { "" };
        let padding = " ".repeat(7usize.saturating_sub(succ.code.len()).max(1));
        let cell = if options.color {
            format!("{ANSI_CYAN}{}{ANSI_RESET}{padding}{}", succ.code, succ.name)
        } else {
            format!("{}{padding}{}", succ.code, succ.name)
        };
        table.add_row(vec![Cell::new(label), Cell::new(&cell)]);
    }

    if options.color {
        dim_borders(&table.to_string())
    } else {
        table.to_string()
    }
}

fn render_roles_table(
    record: &InfoRecord,
    options: &RenderOptions,
    is_record_closed: bool,
) -> (String, Vec<AnomalyType>) {
    // 1. Sort roles
    // 1. active before closed: operational_end.is_none() first
    // 2. start date, ascending: oldest first
    // 3. RO code, numeric: tie-break only (RO76 before RO177)
    let mut sorted_roles = record.roles.clone();
    sorted_roles.sort_by(|a, b| {
        let a_active = a.operational_end.is_none();
        let b_active = b.operational_end.is_none();
        b_active
            .cmp(&a_active)
            .then_with(|| a.operational_start.cmp(&b.operational_start))
            .then_with(|| parse_ro_number(&a.role_code).cmp(&parse_ro_number(&b.role_code)))
            .then_with(|| a.role_code.cmp(&b.role_code))
    });

    let total_active = record
        .roles
        .iter()
        .filter(|r| r.operational_end.is_none())
        .count();
    let total_inactive = record
        .roles
        .iter()
        .filter(|r| r.operational_end.is_some())
        .count();

    // Filter displayed rows
    // On a closed record, default view shows everything. On active record, only active unless --all
    let displayed_roles: Vec<&InfoRole> = if is_record_closed || options.all {
        sorted_roles.iter().collect()
    } else {
        sorted_roles
            .iter()
            .filter(|r| r.operational_end.is_none())
            .collect()
    };

    // Heading counts
    let active_phrase = if total_active == 0 {
        "none active".to_string()
    } else {
        format!("{} active", total_active)
    };
    let heading = if options.style == TableStyle::Markdown {
        format!("## Roles ({}, {} inactive)\n", active_phrase, total_inactive)
    } else {
        format!("Roles ({}, {} inactive)", active_phrase, total_inactive)
    };

    // Anomalies
    let mut anomalies = Vec::new();
    let mut row_anomalies: Vec<Option<AnomalyType>> = Vec::new();
    for r in &displayed_roles {
        let anom = check_anomaly(
            record.operational_end.as_deref(),
            r.operational_start.as_deref(),
            r.operational_end.as_deref(),
        );
        if let Some(a) = anom {
            anomalies.push(a);
        }
        row_anomalies.push(anom);
    }

    // Gutter decision
    // show the gutter if (rows differ in status AND End column absent) OR any row is flagged
    let rows_differ_in_status = displayed_roles
        .iter()
        .any(|r| r.operational_end.is_none())
        && displayed_roles
            .iter()
            .any(|r| r.operational_end.is_some());
    let end_column_absent = !options.band.has_end_date();
    let any_row_flagged = row_anomalies.iter().any(|a| a.is_some());

    let show_gutter = (rows_differ_in_status && end_column_absent) || any_row_flagged;

    let mut table = create_base_table(options.color, options.style);

    // Headers
    let mut headers = Vec::new();
    if show_gutter {
        headers.push("");
    }
    headers.push("Role");
    headers.push("Code");
    if options.band.has_start_date() {
        headers.push("Start");
    }
    if options.band.has_end_date() {
        headers.push("End");
    }
    table.set_header(headers);

    // Set widths
    if options.style == TableStyle::Table {
        match (options.band, show_gutter) {
            (ResponsiveBand::Narrow, true) => apply_column_widths(&mut table, &[1, 42, 5]),
            (ResponsiveBand::Narrow, false) => apply_column_widths(&mut table, &[46, 5]),
            (ResponsiveBand::Medium, true) => apply_column_widths(&mut table, &[1, 49, 5, 10]),
            (ResponsiveBand::Medium, false) => apply_column_widths(&mut table, &[53, 5, 10]),
            (ResponsiveBand::Wide, true) => apply_column_widths(&mut table, &[1, 56, 5, 10, 10]),
            (ResponsiveBand::Wide, false) => apply_column_widths(&mut table, &[60, 5, 10, 10]),
        }
    }

    // Rows
    for (i, r) in displayed_roles.iter().enumerate() {
        let is_row_active = r.operational_end.is_none();
        let anomaly = row_anomalies[i];

        let mut row = Vec::new();

        if show_gutter {
            let gutter_glyph = if anomaly.is_some() {
                "!"
            } else if is_row_active && end_column_absent {
                "●"
            } else {
                " "
            };
            let gutter_text = if options.color {
                if !is_row_active {
                    format!("{ANSI_MUTED}{gutter_glyph}{ANSI_RESET}")
                } else if gutter_glyph == "●" {
                    format!("{ANSI_GREEN}●{ANSI_RESET}")
                } else {
                    gutter_glyph.to_string()
                }
            } else {
                gutter_glyph.to_string()
            };
            row.push(Cell::new(&gutter_text));
        }

        let role_text = if options.color && !is_row_active {
            format!("{ANSI_MUTED}{}{ANSI_RESET}", r.role_name)
        } else {
            r.role_name.clone()
        };
        row.push(Cell::new(&role_text));

        let code_text = if options.color {
            if is_row_active {
                format!("{ANSI_YELLOW}{}{ANSI_RESET}", r.role_code)
            } else {
                format!("{ANSI_MUTED}{}{ANSI_RESET}", r.role_code)
            }
        } else {
            r.role_code.clone()
        };
        row.push(Cell::new(&code_text));

        if options.band.has_start_date() {
            let start_val = r.operational_start.as_deref().unwrap_or("—");
            let start_text = if options.color && !is_row_active {
                format!("{ANSI_MUTED}{start_val}{ANSI_RESET}")
            } else {
                start_val.to_string()
            };
            row.push(Cell::new(&start_text));
        }
        if options.band.has_end_date() {
            let end_val = r.operational_end.as_deref().unwrap_or("—");
            let end_text = if options.color && !is_row_active {
                format!("{ANSI_MUTED}{end_val}{ANSI_RESET}")
            } else {
                end_val.to_string()
            };
            row.push(Cell::new(&end_text));
        }

        table.add_row(row);
    }

    let mut res = heading;
    res.push('\n');
    if options.color {
        res.push_str(&dim_borders(&table.to_string()));
    } else {
        res.push_str(&table.to_string());
    }
    (res, anomalies)
}

fn render_relationships_table(
    record: &InfoRecord,
    options: &RenderOptions,
    is_record_closed: bool,
) -> (String, Vec<AnomalyType>) {
    // 1. Filter to direction == "outbound"
    let mut outbound_rels: Vec<InfoRelationship> = record
        .relationships
        .iter()
        .filter(|r| r.direction == "outbound")
        .cloned()
        .collect();

    // 2. Sort relationships
    // 1. relationship type (canonical RE order)
    // 2. active before closed: operational_end.is_none() first
    // 3. start date, descending: newest first
    // 4. ODS code, ascending: tie-break only
    outbound_rels.sort_by(|a, b| {
        let (rank_a, _) = canonical_re_rank_and_phrase(&a.rel_code);
        let (rank_b, _) = canonical_re_rank_and_phrase(&b.rel_code);
        rank_a
            .cmp(&rank_b)
            .then_with(|| {
                let a_active = a.operational_end.is_none();
                let b_active = b.operational_end.is_none();
                b_active.cmp(&a_active)
            })
            .then_with(|| b.operational_start.cmp(&a.operational_start))
            .then_with(|| a.code.cmp(&b.code))
    });

    let total_active = outbound_rels
        .iter()
        .filter(|r| r.operational_end.is_none())
        .count();
    let total_inactive = outbound_rels
        .iter()
        .filter(|r| r.operational_end.is_some())
        .count();

    // Filter displayed rows
    let displayed_rels: Vec<&InfoRelationship> = if is_record_closed || options.all {
        outbound_rels.iter().collect()
    } else {
        outbound_rels
            .iter()
            .filter(|r| r.operational_end.is_none())
            .collect()
    };

    // Heading counts
    let active_phrase = if total_active == 0 {
        "none active".to_string()
    } else {
        format!("{} active", total_active)
    };
    let heading = if options.style == TableStyle::Markdown {
        format!("## Relationships ({}, {} inactive)\n", active_phrase, total_inactive)
    } else {
        format!("Relationships ({}, {} inactive)", active_phrase, total_inactive)
    };

    // Anomalies
    let mut anomalies = Vec::new();
    let mut row_anomalies: Vec<Option<AnomalyType>> = Vec::new();
    for r in &displayed_rels {
        let anom = check_anomaly(
            record.operational_end.as_deref(),
            r.operational_start.as_deref(),
            r.operational_end.as_deref(),
        );
        if let Some(a) = anom {
            anomalies.push(a);
        }
        row_anomalies.push(anom);
    }

    // Gutter decision
    let rows_differ_in_status = displayed_rels
        .iter()
        .any(|r| r.operational_end.is_none())
        && displayed_rels
            .iter()
            .any(|r| r.operational_end.is_some());
    let end_column_absent = !options.band.has_end_date();
    let any_row_flagged = row_anomalies.iter().any(|a| a.is_some());

    let show_gutter = (rows_differ_in_status && end_column_absent) || any_row_flagged;

    let mut table = create_base_table(options.color, options.style);

    // Headers
    let mut headers = Vec::new();
    if show_gutter {
        headers.push("");
    }
    headers.push("Relationship");
    headers.push("Organisation");
    headers.push("Code");
    if options.band.has_start_date() {
        headers.push("Start");
    }
    if options.band.has_end_date() {
        headers.push("End");
    }
    table.set_header(headers);

    // Set widths: Relationship and Code columns take width from content
    if options.style == TableStyle::Table {
        let max_phrase_len = displayed_rels
            .iter()
            .map(|r| canonical_re_rank_and_phrase(&r.rel_code).1.len())
            .max()
            .unwrap_or(0);
        let rel_width = (max_phrase_len.max("Relationship".len())) as u16;

        let max_code_len = displayed_rels
            .iter()
            .map(|r| r.code.len())
            .max()
            .unwrap_or(0);
        let code_width = (max_code_len.max("Code".len())) as u16;

        match (options.band, show_gutter) {
            (ResponsiveBand::Narrow, true) => {
                let org_width = 44u16.saturating_sub(rel_width).saturating_sub(code_width);
                apply_column_widths(&mut table, &[1, rel_width, org_width, code_width]);
            }
            (ResponsiveBand::Narrow, false) => {
                let org_width = 48u16.saturating_sub(rel_width).saturating_sub(code_width);
                apply_column_widths(&mut table, &[rel_width, org_width, code_width]);
            }
            (ResponsiveBand::Medium, true) => {
                let org_width = 51u16.saturating_sub(rel_width).saturating_sub(code_width);
                apply_column_widths(&mut table, &[1, rel_width, org_width, code_width, 10]);
            }
            (ResponsiveBand::Medium, false) => {
                let org_width = 55u16.saturating_sub(rel_width).saturating_sub(code_width);
                apply_column_widths(&mut table, &[rel_width, org_width, code_width, 10]);
            }
            (ResponsiveBand::Wide, true) => {
                let org_width = 58u16.saturating_sub(rel_width).saturating_sub(code_width);
                apply_column_widths(&mut table, &[1, rel_width, org_width, code_width, 10, 10]);
            }
            (ResponsiveBand::Wide, false) => {
                let org_width = 62u16.saturating_sub(rel_width).saturating_sub(code_width);
                apply_column_widths(&mut table, &[rel_width, org_width, code_width, 10, 10]);
            }
        }
    }

    // Rows with Group Blanking
    let mut prev_rel_code: Option<&str> = None;

    for (i, r) in displayed_rels.iter().enumerate() {
        let is_row_active = r.operational_end.is_none();
        let anomaly = row_anomalies[i];

        let mut row = Vec::new();

        if show_gutter {
            let gutter_glyph = if anomaly.is_some() {
                "!"
            } else if is_row_active && end_column_absent {
                "●"
            } else {
                " "
            };
            let gutter_text = if options.color {
                if !is_row_active {
                    format!("{ANSI_MUTED}{gutter_glyph}{ANSI_RESET}")
                } else if gutter_glyph == "●" {
                    format!("{ANSI_GREEN}●{ANSI_RESET}")
                } else {
                    gutter_glyph.to_string()
                }
            } else {
                gutter_glyph.to_string()
            };
            row.push(Cell::new(&gutter_text));
        }

        // Group blanking on relationship phrase
        let is_first_of_group = prev_rel_code != Some(&r.rel_code);
        prev_rel_code = Some(&r.rel_code);

        let phrase = if is_first_of_group {
            let (_, p) = canonical_re_rank_and_phrase(&r.rel_code);
            p
        } else {
            ""
        };
        let phrase_text = if options.color && !is_row_active && !phrase.is_empty() {
            format!("{ANSI_MUTED}{phrase}{ANSI_RESET}")
        } else {
            phrase.to_string()
        };
        row.push(Cell::new(&phrase_text));

        // Organisation cell: just name without leading code or padding
        let org_text = if options.color && !is_row_active {
            format!("{ANSI_MUTED}{}{ANSI_RESET}", r.name)
        } else {
            r.name.clone()
        };
        row.push(Cell::new(&org_text));

        // Code cell: target org code
        let code_text = if options.color {
            if is_row_active {
                format!("{ANSI_CYAN}{}{ANSI_RESET}", r.code)
            } else {
                format!("{ANSI_MUTED}{}{ANSI_RESET}", r.code)
            }
        } else {
            r.code.clone()
        };
        row.push(Cell::new(&code_text));

        if options.band.has_start_date() {
            let start_val = r.operational_start.as_deref().unwrap_or("—");
            let start_text = if options.color && !is_row_active {
                format!("{ANSI_MUTED}{start_val}{ANSI_RESET}")
            } else {
                start_val.to_string()
            };
            row.push(Cell::new(&start_text));
        }
        if options.band.has_end_date() {
            let end_val = r.operational_end.as_deref().unwrap_or("—");
            let end_text = if options.color && !is_row_active {
                format!("{ANSI_MUTED}{end_val}{ANSI_RESET}")
            } else {
                end_val.to_string()
            };
            row.push(Cell::new(&end_text));
        }

        table.add_row(row);
    }

    if options.style == TableStyle::Table {
        for row in table.row_iter_mut() {
            row.max_height(1);
        }
    }

    let mut res = heading;
    res.push('\n');
    if options.color {
        res.push_str(&dim_borders(&table.to_string()));
    } else {
        res.push_str(&table.to_string());
    }
    (res, anomalies)
}
