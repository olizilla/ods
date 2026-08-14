use anyhow::{Context, Result};
use arrow::array::{Array, StringArray};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::stdout;
use std::path::{Path, PathBuf};

use crate::commands::find;

#[derive(Debug, Clone)]
pub struct OrgItem {
    pub ods_code: String,
    pub name: String,
    pub record_class: String,
    pub status: String,
    pub role: String,
    pub role_code: String,
    pub address: String,
    pub town: String,
    pub postcode: String,
    pub telephone: String,
    pub icb: String,
    pub icb_code: String,
    pub trust: String,
    pub trust_code: String,
    pub pcn: String,
    pub pcn_code: String,
    pub region: String,
    pub region_code: String,
    pub country: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSummary {
    pub region_code: String,
    pub region_name: String,
    pub country: String,
    pub total_entities: usize,
    pub active_entities: usize,
    pub icb_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcbSummary {
    pub icb_code: String,
    pub icb_name: String,
    pub region_code: String,
    pub total_entities: usize,
    pub active_entities: usize,
    pub trust_count: usize,
    pub pcn_count: usize,
    pub practice_count: usize,
}

/// Standalone, fully testable hierarchy & navigation model engine.
pub struct HierarchyEngine {
    all_records: Vec<OrgItem>,
    regions: Vec<RegionSummary>,
    icb_summaries: Vec<IcbSummary>,
}

impl HierarchyEngine {
    pub fn new(records: Vec<OrgItem>) -> Self {
        let regions = build_region_summaries(&records);
        let icb_summaries = build_icb_summaries(&records);
        Self {
            all_records: records,
            regions,
            icb_summaries,
        }
    }

    pub fn regions(&self) -> &[RegionSummary] {
        &self.regions
    }

    pub fn icb_summaries(&self) -> &[IcbSummary] {
        &self.icb_summaries
    }

    pub fn all_records(&self) -> &[OrgItem] {
        &self.all_records
    }

    pub fn get_icbs_for_region(&self, region_code: &str) -> Vec<usize> {
        self.icb_summaries
            .iter()
            .enumerate()
            .filter(|(_, icb)| match_region_code(&icb.region_code, region_code))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn search(&self, query: &str) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.all_records
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.ods_code.to_lowercase().contains(&q)
                    || r.name.to_lowercase().contains(&q)
                    || r.postcode.to_lowercase().contains(&q)
                    || r.town.to_lowercase().contains(&q)
                    || r.icb.to_lowercase().contains(&q)
                    || r.trust.to_lowercase().contains(&q)
                    || r.role.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NavLevel {
    Level1Regions,
    Level2Icbs,
    Level3Search,
}

#[derive(Debug, PartialEq, Eq)]
enum FocusPane {
    SearchInput,
    ResultsList,
}

pub struct AppState {
    query: String,
    engine: HierarchyEngine,
    selected_region_code: Option<String>,
    nav_level: NavLevel,
    filtered_indices: Vec<usize>,
    list_state: ListState,
    focus: FocusPane,
    status_msg: Option<String>,
    release_date: String,
}

impl AppState {
    pub fn new(data_dir: PathBuf, initial_query: Option<String>) -> Result<Self> {
        let (records, release_date) = load_dataset(&data_dir)?;
        let engine = HierarchyEngine::new(records);
        let query = initial_query.unwrap_or_default();

        let mut app = Self {
            query,
            engine,
            selected_region_code: None,
            nav_level: NavLevel::Level1Regions,
            filtered_indices: Vec::new(),
            list_state: ListState::default(),
            focus: FocusPane::SearchInput,
            status_msg: None,
            release_date,
        };
        app.update_filter();
        Ok(app)
    }

    fn update_filter(&mut self) {
        let q = self.query.trim().to_lowercase();
        if !q.is_empty() {
            // Global Search Mode across 158,000 entities
            self.nav_level = NavLevel::Level3Search;
            self.filtered_indices = self.engine.search(&q);
        } else if let Some(ref reg_code) = self.selected_region_code {
            // Level 2: ICBs in selected Region
            self.nav_level = NavLevel::Level2Icbs;
            self.filtered_indices = self.engine.get_icbs_for_region(reg_code);
        } else {
            // Level 1: Nations & NHS Regions
            self.nav_level = NavLevel::Level1Regions;
            self.filtered_indices = (0..self.engine.regions().len()).collect();
        }

        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            let current = self.list_state.selected().unwrap_or(0);
            if current >= self.filtered_indices.len() {
                self.list_state.select(Some(self.filtered_indices.len() - 1));
            } else {
                self.list_state.select(Some(current));
            }
        }
    }

    fn selected_region(&self) -> Option<&RegionSummary> {
        if self.nav_level != NavLevel::Level1Regions {
            return None;
        }
        let idx = self.list_state.selected()?;
        let orig_idx = *self.filtered_indices.get(idx)?;
        self.engine.regions().get(orig_idx)
    }

    fn selected_icb(&self) -> Option<&IcbSummary> {
        if self.nav_level != NavLevel::Level2Icbs {
            return None;
        }
        let idx = self.list_state.selected()?;
        let orig_idx = *self.filtered_indices.get(idx)?;
        self.engine.icb_summaries().get(orig_idx)
    }

    fn selected_record(&self) -> Option<&OrgItem> {
        if self.nav_level != NavLevel::Level3Search {
            return None;
        }
        let idx = self.list_state.selected()?;
        let orig_idx = *self.filtered_indices.get(idx)?;
        self.engine.all_records().get(orig_idx)
    }

    fn next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i + 1 >= self.filtered_indices.len() {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_indices.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }
}

fn match_region_code(icb_reg: &str, target_reg: &str) -> bool {
    if target_reg == "WALES" {
        icb_reg.contains("WALES") || target_reg.contains(icb_reg)
    } else if target_reg == "SCOTLAND" {
        icb_reg.contains("SCOTLAND") || target_reg.contains(icb_reg)
    } else if target_reg == "NI" {
        icb_reg.contains("NORTHERN IRELAND") || icb_reg.contains("NI")
    } else {
        icb_reg == target_reg
    }
}

fn is_statutory_icb(name: &str) -> bool {
    let u = name.to_uppercase();
    u.starts_with("NHS ") && u.ends_with("INTEGRATED CARE BOARD")
}

fn clean_icb_name(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let s = raw.split(" - ").next().unwrap_or(raw).trim();
    if let Some(stripped) = s.strip_suffix(" ICB") {
        format!("{} INTEGRATED CARE BOARD", stripped)
    } else {
        s.to_string()
    }
}

fn build_region_summaries(records: &[OrgItem]) -> Vec<RegionSummary> {
    struct Acc {
        code: String,
        name: String,
        country: String,
        total: usize,
        active: usize,
        icbs: std::collections::HashSet<String>,
    }

    let mut map: BTreeMap<String, Acc> = BTreeMap::new();

    let eng_regions = vec![
        ("Y56", "London Region", "ENGLAND"),
        ("Y59", "South East Region", "ENGLAND"),
        ("Y58", "South West Region", "ENGLAND"),
        ("Y60", "Midlands Region", "ENGLAND"),
        ("Y61", "East of England Region", "ENGLAND"),
        ("Y62", "North West Region", "ENGLAND"),
        ("Y63", "North East and Yorkshire Region", "ENGLAND"),
        ("WALES", "Wales (Local Health Boards)", "WALES"),
        ("SCOTLAND", "Scotland (NHS Health Boards)", "SCOTLAND"),
        ("NI", "Northern Ireland (HSC Trusts)", "NORTHERN IRELAND"),
        ("UNMAPPED", "Unmapped / National Entities", "UK"),
    ];

    for (code, name, country) in eng_regions {
        map.insert(
            code.to_string(),
            Acc {
                code: code.to_string(),
                name: name.to_string(),
                country: country.to_string(),
                total: 0,
                active: 0,
                icbs: std::collections::HashSet::new(),
            },
        );
    }

    for r in records {
        let key = if r.country.eq_ignore_ascii_case("WALES") {
            "WALES"
        } else if r.country.eq_ignore_ascii_case("SCOTLAND") {
            "SCOTLAND"
        } else if r.country.eq_ignore_ascii_case("NORTHERN IRELAND") {
            "NI"
        } else if !r.region_code.is_empty() && map.contains_key(&r.region_code) {
            &r.region_code
        } else if !r.region.is_empty() {
            match r.region.to_uppercase().as_str() {
                s if s.contains("LONDON") => "Y56",
                s if s.contains("SOUTH EAST") => "Y59",
                s if s.contains("SOUTH WEST") => "Y58",
                s if s.contains("MIDLANDS") => "Y60",
                s if s.contains("EAST OF ENGLAND") => "Y61",
                s if s.contains("NORTH WEST") => "Y62",
                s if s.contains("YORKSHIRE") => "Y63",
                _ => "UNMAPPED",
            }
        } else {
            "UNMAPPED"
        };

        if let Some(entry) = map.get_mut(key) {
            entry.total += 1;
            if r.is_active {
                entry.active += 1;
            }
            if !r.icb_code.is_empty() {
                entry.icbs.insert(r.icb_code.clone());
            }
        }
    }

    map.into_values()
        .map(|acc| RegionSummary {
            region_code: acc.code,
            region_name: acc.name,
            country: acc.country,
            total_entities: acc.total,
            active_entities: acc.active,
            icb_count: acc.icbs.len(),
        })
        .collect()
}

fn build_icb_summaries(records: &[OrgItem]) -> Vec<IcbSummary> {
    struct Acc {
        icb_code: String,
        icb_name: String,
        region_code: String,
        total: usize,
        active: usize,
        trusts: std::collections::HashSet<String>,
        pcns: std::collections::HashSet<String>,
        practices: usize,
    }

    let mut map: BTreeMap<String, Acc> = BTreeMap::new();

    // 1. First pass: Seed map with all Statutory ICB bodies found in dataset
    for r in records {
        if r.is_active && is_statutory_icb(&r.name) {
            let name = clean_icb_name(&r.name);
            let reg_code = if !r.region_code.is_empty() { r.region_code.clone() } else { "Y56".to_string() };
            map.entry(name.clone()).or_insert_with(|| Acc {
                icb_code: r.ods_code.clone(),
                icb_name: name,
                region_code: reg_code,
                total: 0,
                active: 0,
                trusts: std::collections::HashSet::new(),
                pcns: std::collections::HashSet::new(),
                practices: 0,
            });
        }
    }

    // 2. Second pass: Aggregate all child organisations into their parent Statutory ICB
    for r in records {
        if r.icb.is_empty() {
            continue;
        }
        let cleaned = clean_icb_name(&r.icb);
        let matched_key = map.keys().find(|k| k.contains(&cleaned) || cleaned.contains(k.as_str())).cloned();

        if let Some(key) = matched_key {
            if let Some(entry) = map.get_mut(&key) {
                entry.total += 1;
                if r.is_active {
                    entry.active += 1;
                }
                if !r.trust.is_empty() {
                    entry.trusts.insert(r.trust_code.clone());
                }
                if !r.pcn.is_empty() {
                    entry.pcns.insert(r.pcn_code.clone());
                }
                if r.role.to_lowercase().contains("practice") || r.role.to_lowercase().contains("prescribing") {
                    entry.practices += 1;
                }
            }
        }
    }

    map.into_values()
        .map(|acc| IcbSummary {
            icb_code: acc.icb_code,
            icb_name: acc.icb_name,
            region_code: acc.region_code,
            total_entities: acc.total,
            active_entities: acc.active,
            trust_count: acc.trusts.len(),
            pcn_count: acc.pcns.len(),
            practice_count: acc.practices,
        })
        .collect()
}

fn load_dataset(data_dir: &Path) -> Result<(Vec<OrgItem>, String)> {
    let orgs_file = data_dir.join("orgs.parquet");
    if !orgs_file.exists() {
        anyhow::bail!("Missing orgs.parquet in {}", data_dir.display());
    }

    let file = File::open(&orgs_file).with_context(|| format!("opening {}", orgs_file.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut records = Vec::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();

        let get_str_column = |col_name: &str| -> Option<&StringArray> {
            schema
                .index_of(col_name)
                .ok()
                .and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>())
        };

        let ods_code_arr = get_str_column("ods_code").context("missing ods_code")?;
        let name_arr = get_str_column("name").context("missing name")?;
        let record_class_arr = get_str_column("entity_type");
        let status_arr = get_str_column("status");
        let role_code_arr = get_str_column("primary_role_code");
        let address_arr = get_str_column("address");
        let town_arr = get_str_column("town");
        let postcode_arr = get_str_column("postcode");
        let telephone_arr = get_str_column("telephone");
        let icb_arr = get_str_column("icb_name");
        let icb_code_arr = get_str_column("icb_code");
        let trust_arr = get_str_column("trust_name");
        let trust_code_arr = get_str_column("trust_code");
        let pcn_arr = get_str_column("pcn_name");
        let pcn_code_arr = get_str_column("pcn_code");
        let region_arr = get_str_column("region_name");
        let region_code_arr = get_str_column("region_code");
        let country_arr = get_str_column("country");

        for i in 0..batch.num_rows() {
            let ods_code = ods_code_arr.value(i).to_string();
            let name = name_arr.value(i).to_string();
            let record_class = record_class_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let status = status_arr.map(|a| a.value(i)).unwrap_or("Active").to_string();
            let role_code = role_code_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            // orgs.parquet stores the role code; resolve the curated name for display.
            let role = crate::roles::role_names()
                .name(&role_code)
                .unwrap_or(&role_code)
                .to_string();
            let address = address_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let town = town_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let postcode = postcode_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let telephone = telephone_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let icb = icb_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let icb_code = icb_code_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let trust = trust_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let trust_code = trust_code_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let pcn = pcn_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let pcn_code = pcn_code_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let region = region_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let region_code = region_code_arr.map(|a| a.value(i)).unwrap_or("").to_string();
            let country = country_arr.map(|a| a.value(i)).unwrap_or("ENGLAND").to_string();
            let is_active = status.eq_ignore_ascii_case("active");

            records.push(OrgItem {
                ods_code,
                name,
                record_class,
                status,
                role,
                role_code,
                address,
                town,
                postcode,
                telephone,
                icb,
                icb_code,
                trust,
                trust_code,
                pcn,
                pcn_code,
                region,
                region_code,
                country,
                is_active,
            });
        }
    }

    let release_date = data_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("current")
        .to_string();

    Ok((records, release_date))
}

pub fn run(args: find::Args) -> Result<()> {
    let data_dir = args.input.clone();
    if !data_dir.join("orgs.parquet").exists() {
        eprintln!("Initialising workspace dataset...");
        crate::commands::pull::run(crate::commands::pull::Args {
            release_date: None,
            list: false,
            force: false,
            api_key: None,
            verbose: false,
        })?;
    }

    let resolved_dir = crate::workspace::discover_parquet_dir(Some(&data_dir))?;
    let mut app = AppState::new(resolved_dir, args.query)?;

    enable_raw_mode().context("failed to enable terminal raw mode")?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    res
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break;
                }

                match key.code {
                    KeyCode::Esc => {
                        if !app.query.is_empty() {
                            app.query.clear();
                            app.update_filter();
                        } else if app.selected_region_code.is_some() {
                            app.selected_region_code = None;
                            app.update_filter();
                        } else {
                            break;
                        }
                    }
                    KeyCode::Enter => {
                        if app.nav_level == NavLevel::Level1Regions {
                            if let Some(reg) = app.selected_region().cloned() {
                                app.selected_region_code = Some(reg.region_code);
                                app.update_filter();
                            }
                        } else if app.nav_level == NavLevel::Level2Icbs {
                            if let Some(icb) = app.selected_icb().cloned() {
                                app.query = icb.icb_code.clone();
                                app.update_filter();
                            }
                        }
                    }
                    KeyCode::Char('q') if app.focus == FocusPane::ResultsList => break,
                    KeyCode::Down => app.next(),
                    KeyCode::Up => app.previous(),
                    KeyCode::Tab => {
                        app.focus = match app.focus {
                            FocusPane::SearchInput => FocusPane::ResultsList,
                            FocusPane::ResultsList => FocusPane::SearchInput,
                        };
                    }
                    KeyCode::Char('c') if app.focus == FocusPane::ResultsList => {
                        if let Some(rec) = app.selected_record() {
                            app.status_msg = Some(format!("Copied ODS Code {}", rec.ods_code));
                        } else if let Some(icb) = app.selected_icb() {
                            app.status_msg = Some(format!("Copied ICB Code {}", icb.icb_code));
                        }
                    }
                    KeyCode::Backspace => {
                        if app.focus == FocusPane::SearchInput {
                            app.query.pop();
                            app.update_filter();
                        } else if app.selected_region_code.is_some() {
                            app.selected_region_code = None;
                            app.update_filter();
                        }
                    }
                    KeyCode::Char(ch) => {
                        if app.focus == FocusPane::SearchInput {
                            app.query.push(ch);
                            app.update_filter();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn ui(f: &mut Frame, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search bar
            Constraint::Min(10),   // Split view (results + inspector)
            Constraint::Length(1), // Footer status line
        ])
        .split(f.area());

    // 1. Search Bar
    let input_style = if app.focus == FocusPane::SearchInput {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let breadcrumb = match app.nav_level {
        NavLevel::Level1Regions => "Level 1: UK Nations & Regions (Select & press [Enter])",
        NavLevel::Level2Icbs => "Level 2: ICBs & Health Boards in Region (Select & press [Enter] to view entities)",
        NavLevel::Level3Search => "Global Search Mode",
    };

    let search_bar = Paragraph::new(Line::from(vec![
        Span::styled(" Filter / Search [/]: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(
            if app.query.is_empty() { breadcrumb } else { &app.query },
            if app.query.is_empty() { Style::default().fg(Color::DarkGray) } else { input_style },
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" ods find — UK NHS Hierarchy Explorer (Release: {}) ", app.release_date)),
    );
    f.render_widget(search_bar, chunks[0]);

    // 2. Main Body Split (Left 44%, Right 56%)
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(chunks[1]);

    match app.nav_level {
        NavLevel::Level1Regions => {
            // --- Level 1: Regions & Nations List (Left) ---
            let list_items: Vec<ListItem> = app
                .filtered_indices
                .iter()
                .filter_map(|&orig_idx| app.engine.regions().get(orig_idx))
                .map(|reg| {
                    let line = Line::from(vec![
                        Span::styled(format!("{:<8} ", reg.region_code), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:<30} ", truncate(&reg.region_name, 30)), Style::default().fg(Color::White)),
                        Span::styled(format!("({} orgs)", reg.active_entities), Style::default().fg(Color::DarkGray)),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let list_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" Level 1: UK Nations & Regions ({}) ", app.engine.regions().len()))
                .style(if app.focus == FocusPane::ResultsList {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                });

            let results_list = List::new(list_items)
                .block(list_block)
                .highlight_style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            f.render_stateful_widget(results_list, body_chunks[0], &mut app.list_state);

            // Inspector (Right)
            let inspector_block = Block::default().borders(Borders::ALL).title(" Region / Nation Summary ");
            if let Some(reg) = app.selected_region() {
                let mut text = Vec::new();
                text.push(Line::from(vec![
                    Span::styled("Region Name:  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(&reg.region_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("Code:         ", Style::default().fg(Color::Cyan)),
                    Span::styled(&reg.region_code, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw("  Country: "),
                    Span::styled(&reg.country, Style::default().fg(Color::Green)),
                ]));
                text.push(Line::raw(""));
                text.push(Line::styled("--- Active Entities ---", Style::default().fg(Color::DarkGray)));
                text.push(Line::from(vec![
                    Span::styled("Active Orgs:  ", Style::default().fg(Color::Cyan)),
                    Span::styled(format!("{}", reg.active_entities), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("ICBs / Boards:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{}", reg.icb_count)),
                ]));
                text.push(Line::raw(""));
                text.push(Line::styled("💡 Press [Enter] to view ICBs & Health Boards in this region.", Style::default().fg(Color::Yellow)));

                let paragraph = Paragraph::new(text).block(inspector_block).wrap(Wrap { trim: true });
                f.render_widget(paragraph, body_chunks[1]);
            } else {
                let empty = Paragraph::new("No region selected").block(inspector_block);
                f.render_widget(empty, body_chunks[1]);
            }
        }
        NavLevel::Level2Icbs => {
            // --- Level 2: ICBs List in Selected Region (Left) ---
            let list_items: Vec<ListItem> = app
                .filtered_indices
                .iter()
                .filter_map(|&orig_idx| app.engine.icb_summaries().get(orig_idx))
                .map(|icb| {
                    let line = Line::from(vec![
                        Span::styled(format!("{:<6} ", icb.icb_code), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:<28} ", truncate(&icb.icb_name, 28)), Style::default().fg(Color::White)),
                        Span::styled(format!("({} orgs)", icb.active_entities), Style::default().fg(Color::DarkGray)),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let list_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" Level 2: ICBs & Health Boards ({}) ", app.filtered_indices.len()))
                .style(if app.focus == FocusPane::ResultsList {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                });

            let results_list = List::new(list_items)
                .block(list_block)
                .highlight_style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            f.render_stateful_widget(results_list, body_chunks[0], &mut app.list_state);

            // Inspector (Right)
            let inspector_block = Block::default().borders(Borders::ALL).title(" Integrated Care Board Summary ");
            if let Some(icb) = app.selected_icb() {
                let mut text = Vec::new();
                text.push(Line::from(vec![
                    Span::styled("ICB Name:     ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(&icb.icb_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("Code:         ", Style::default().fg(Color::Cyan)),
                    Span::styled(&icb.icb_code, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ]));
                text.push(Line::raw(""));
                text.push(Line::styled("--- Commissioned Services ---", Style::default().fg(Color::DarkGray)));
                text.push(Line::from(vec![
                    Span::styled("Active Entities:    ", Style::default().fg(Color::Cyan)),
                    Span::styled(format!("{}", icb.active_entities), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("GP Practices:       ", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{}", icb.practice_count)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("NHS Trusts:         ", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{}", icb.trust_count)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("Primary Care Nets:  ", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{}", icb.pcn_count)),
                ]));
                text.push(Line::raw(""));
                text.push(Line::styled("💡 Press [Enter] to view all child entities under this ICB.", Style::default().fg(Color::Yellow)));

                let paragraph = Paragraph::new(text).block(inspector_block).wrap(Wrap { trim: true });
                f.render_widget(paragraph, body_chunks[1]);
            } else {
                let empty = Paragraph::new("No ICB selected").block(inspector_block);
                f.render_widget(empty, body_chunks[1]);
            }
        }
        NavLevel::Level3Search => {
            // --- Level 3: Search Results List (Left) ---
            let list_items: Vec<ListItem> = app
                .filtered_indices
                .iter()
                .filter_map(|&orig_idx| app.engine.all_records().get(orig_idx))
                .map(|rec| {
                    let role_short = if rec.role_code.is_empty() { &rec.role } else { &rec.role_code };
                    let line = Line::from(vec![
                        Span::styled(format!("{:<6} ", rec.ods_code), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:<26} ", truncate(&rec.name, 26)), Style::default().fg(Color::White)),
                        Span::styled(format!("[{}]", role_short), Style::default().fg(Color::DarkGray)),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let list_title = format!(" Search Results ({}) ", app.filtered_indices.len());
            let list_block = Block::default()
                .borders(Borders::ALL)
                .title(list_title)
                .style(if app.focus == FocusPane::ResultsList {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                });

            let results_list = List::new(list_items)
                .block(list_block)
                .highlight_style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            f.render_stateful_widget(results_list, body_chunks[0], &mut app.list_state);

            // Inspector Panel (Right)
            let inspector_block = Block::default().borders(Borders::ALL).title(" Entity Details & Provenance ");
            if let Some(rec) = app.selected_record() {
                let mut text = Vec::new();
                text.push(Line::from(vec![
                    Span::styled("Name:         ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(&rec.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]));
                text.push(Line::from(vec![
                    Span::styled("ODS Code:     ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(&rec.ods_code, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(
                        if rec.is_active { "[ACTIVE]" } else { "[INACTIVE]" },
                        if rec.is_active { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) },
                    ),
                ]));
                text.push(Line::from(vec![
                    Span::styled("Record Class: ", Style::default().fg(Color::Cyan)),
                    Span::raw(&rec.record_class),
                ]));

                if !rec.role.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Primary Role: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format!("{} ({})", rec.role, rec.role_code)),
                    ]));
                }

                text.push(Line::raw(""));
                text.push(Line::styled("--- Location & Contact ---", Style::default().fg(Color::DarkGray)));
                if !rec.address.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Address:      ", Style::default().fg(Color::Cyan)),
                        Span::raw(&rec.address),
                    ]));
                }
                if !rec.town.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Town:         ", Style::default().fg(Color::Cyan)),
                        Span::raw(&rec.town),
                    ]));
                }
                if !rec.postcode.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Postcode:     ", Style::default().fg(Color::Cyan)),
                        Span::raw(&rec.postcode),
                    ]));
                }
                if !rec.telephone.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Telephone:    ", Style::default().fg(Color::Cyan)),
                        Span::raw(&rec.telephone),
                    ]));
                }

                text.push(Line::raw(""));
                text.push(Line::styled("--- Commissioning & Region Links ---", Style::default().fg(Color::DarkGray)));
                if !rec.region.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Region:       ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{} ({})", rec.region, rec.region_code),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                }
                if !rec.icb.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("ICB:          ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{} ({})", rec.icb, rec.icb_code),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                }
                if !rec.trust.is_empty() {
                    text.push(Line::from(vec![
                        Span::styled("Trust:        ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{} ({})", rec.trust, rec.trust_code),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                }

                let paragraph = Paragraph::new(text).block(inspector_block).wrap(Wrap { trim: true });
                f.render_widget(paragraph, body_chunks[1]);
            } else {
                let empty = Paragraph::new("No entity selected").block(inspector_block);
                f.render_widget(empty, body_chunks[1]);
            }
        }
    }

    // 3. Footer Bar
    let footer_text = if let Some(ref msg) = app.status_msg {
        format!(" {} | [Tab] Focus | [↑/↓] Select | [Enter] Drill Down | [Esc/Backspace] Back/Quit", msg)
    } else {
        " [Tab] Focus | [↑/↓] Select | [Enter] Drill Down | [Esc/Backspace] Back/Quit".to_string()
    };
    let footer = Paragraph::new(Span::styled(footer_text, Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[2]);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max - 1])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hierarchy_engine_region_counts() {
        let sample_records = vec![
            OrgItem {
                ods_code: "A101".to_string(),
                name: "London GP Practice".to_string(),
                record_class: "org".to_string(),
                status: "Active".to_string(),
                role: "prescribing cost centre".to_string(),
                role_code: "RO177".to_string(),
                address: "".to_string(),
                town: "London".to_string(),
                postcode: "SW1A 1AA".to_string(),
                telephone: "".to_string(),
                icb: "NHS North Central London ICB".to_string(),
                icb_code: "QMJ".to_string(),
                trust: "".to_string(),
                trust_code: "".to_string(),
                pcn: "".to_string(),
                pcn_code: "".to_string(),
                region: "London Region".to_string(),
                region_code: "Y56".to_string(),
                country: "ENGLAND".to_string(),
                is_active: true,
            },
            OrgItem {
                ods_code: "B202".to_string(),
                name: "Manchester Practice".to_string(),
                record_class: "org".to_string(),
                status: "Active".to_string(),
                role: "prescribing cost centre".to_string(),
                role_code: "RO177".to_string(),
                address: "".to_string(),
                town: "Manchester".to_string(),
                postcode: "M1 1AA".to_string(),
                telephone: "".to_string(),
                icb: "NHS Greater Manchester ICB".to_string(),
                icb_code: "QOP".to_string(),
                trust: "".to_string(),
                trust_code: "".to_string(),
                pcn: "".to_string(),
                pcn_code: "".to_string(),
                region: "North West Region".to_string(),
                region_code: "Y62".to_string(),
                country: "ENGLAND".to_string(),
                is_active: true,
            },
        ];

        let engine = HierarchyEngine::new(sample_records);
        let regions = engine.regions();

        let london = regions.iter().find(|r| r.region_code == "Y56").unwrap();
        assert_eq!(london.active_entities, 1);

        let north_west = regions.iter().find(|r| r.region_code == "Y62").unwrap();
        assert_eq!(north_west.active_entities, 1);

        let unmapped = regions.iter().find(|r| r.region_code == "UNMAPPED").unwrap();
        assert_eq!(unmapped.active_entities, 0);
    }
}
