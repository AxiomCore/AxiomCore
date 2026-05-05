use crate::auth_store::AuthData;
use axiom_lib::action::{Action, StepStatus, ViewMode};
use axiom_lib::contract::AxiomFile;
use axiom_lib::ir::IR;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

pub struct State {
    pub auth: AuthData,
    pub linked_project_id: Option<String>,
    pub current_directory: PathBuf,

    // Build context
    pub is_building: bool,
    pub build_context: BuildContext,
    pub current_step_idx: usize,

    // Inspect context
    pub inspect_context: InspectContext,

    // Watch context
    pub is_rebuilding: bool,
    pub watch_diff: IRDiff,
    pub last_sync_time: String,
    pub watch_build_enabled: bool,
    pub last_schema_hash: String,
    pub previous_ir: Option<IR>,

    pub init_context: InitContext,
    pub pull_context: PullContext,
    pub login_context: LoginContext,

    pub active_view: ViewMode,
}

pub struct BuildStep {
    pub name: String,
    pub status: StepStatus,
}

pub struct BuildContext {
    pub project_id: String,
    pub version: String,
    pub variant: String,
    pub steps: Vec<BuildStep>,
    pub endpoints_count: usize,
    pub schema_hash: String,
    pub logs: VecDeque<String>,
}

pub enum InspectTab {
    Endpoints,
    Models,
}

pub struct InspectContext {
    pub contract: Option<AxiomFile>,
    pub selected_endpoint_idx: usize,
    pub selected_model_idx: usize,
    pub filter_query: String,
    pub active_tab: InspectTab,
}

#[derive(Default)]
pub struct IRDiff {
    pub added_endpoints: Vec<String>,
    pub removed_endpoints: Vec<String>,
    pub modified_policies: Vec<String>,
}

impl IRDiff {
    pub fn from_irs(old: &Option<IR>, new: &IR) -> Self {
        let mut diff = IRDiff::default();
        let Some(old_ir) = old else { return diff };

        let old_map: HashMap<String, u32> = old_ir
            .endpoints
            .values()
            .map(|e| (e.path.clone(), e.id))
            .collect();
        let new_map: HashMap<String, u32> = new
            .endpoints
            .values()
            .map(|e| (e.path.clone(), e.id))
            .collect();

        for path in new_map.keys() {
            if !old_map.contains_key(path) {
                diff.added_endpoints.push(path.clone());
            }
        }
        for path in old_map.keys() {
            if !new_map.contains_key(path) {
                diff.removed_endpoints.push(path.clone());
            }
        }
        diff
    }
}

impl State {
    pub fn new() -> Self {
        let auth = crate::auth_store::load_auth_data().unwrap_or_default();
        let current_dir = std::env::current_dir().unwrap_or_default();
        let linked_project_id = crate::auth_store::get_project_id(&current_dir)
            .ok()
            .flatten();

        Self {
            auth,
            linked_project_id: linked_project_id.clone(),
            current_directory: current_dir,
            is_building: false,
            current_step_idx: 0,
            pull_context: PullContext {
                step: PullStep::SourceSelection,
                source_mode: 0,
                selected_framework: 0,
                local_file_path: String::new(),
                available_projects: Vec::new(),
                selected_project_idx: 0,
            },
            init_context: InitContext::default(),
            active_view: ViewMode::Dashboard,
            login_context: LoginContext {
                status: LoginStatus::Idle,
            },
            build_context: BuildContext {
                project_id: linked_project_id.unwrap_or_default(),
                version: "v0.0.0".into(),
                variant: "default".into(),
                steps: vec![
                    BuildStep {
                        name: "Evaluating Pkl".into(),
                        status: StepStatus::Waiting,
                    },
                    BuildStep {
                        name: "Introspecting Backend".into(),
                        status: StepStatus::Waiting,
                    },
                    BuildStep {
                        name: "Generating Schema".into(),
                        status: StepStatus::Waiting,
                    },
                    BuildStep {
                        name: "Packaging".into(),
                        status: StepStatus::Waiting,
                    },
                ],
                endpoints_count: 0,
                schema_hash: "N/A".into(),
                logs: VecDeque::with_capacity(50),
            },
            inspect_context: InspectContext {
                contract: None,
                selected_endpoint_idx: 0,
                selected_model_idx: 0,
                filter_query: "".into(),
                active_tab: InspectTab::Endpoints,
            },
            is_rebuilding: false,
            watch_diff: IRDiff::default(),
            last_sync_time: "Never".into(),
            watch_build_enabled: false,
            last_schema_hash: "N/A".into(),
            previous_ir: None,
        }
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::UpdateBuildStep {
                index,
                status,
                message,
            } => {
                if let Some(step) = self.build_context.steps.get_mut(index) {
                    step.status = status;
                }
                self.current_step_idx = index;
                self.build_context.logs.push_back(message);
            }
            Action::UpdateBuildStats {
                endpoints,
                schema_hash,
            } => {
                self.build_context.endpoints_count = endpoints;
                self.build_context.schema_hash = schema_hash;
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InitStep {
    Language,
    Framework,
    ProjectName,
    Entrypoint,
    Success,
}

#[derive(Debug, Clone)]
pub struct InitContext {
    pub step: InitStep,
    pub languages: Vec<String>, // Dynamic list for auto-sorting
    pub selected_language: usize,
    pub selected_framework: usize,
    pub project_name: String,
    pub entrypoint: String,
    pub cursor_position: usize, // Tracks cursor for the active text input
    pub current_step: usize,    // e.g. 1
    pub total_steps: usize,     // e.g. 3 (Go) or 4 (Python)
}

impl Default for InitContext {
    fn default() -> Self {
        Self {
            step: InitStep::Language,
            languages: vec![
                "Python".into(),
                "Go".into(),
                "Rust (Coming soon)".into(),
                "Javascript (Coming soon)".into(),
                "Dart (Coming soon)".into(),
            ],
            selected_language: 0,
            selected_framework: 0,
            project_name: String::new(),
            entrypoint: String::new(),
            cursor_position: 0,
            current_step: 1,
            total_steps: 4,
        }
    }
}

pub enum LoginStatus {
    Idle,
    WaitingForUser { code: String, url: String },
    Verifying,
    Success,
    Error(String),
}

pub struct LoginContext {
    pub status: LoginStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PullStep {
    SourceSelection,   // Remote vs Local
    ProjectLink,       // If Remote: Select Project
    LocalPathInput,    // If Local: Paste path
    FrontendSelection, // Flutter, Swift, etc.
    Processing,
    Success,
}

pub struct PullContext {
    pub step: PullStep,
    pub source_mode: usize, // 0 = Remote, 1 = Local
    pub selected_framework: usize,
    pub local_file_path: String,
    pub available_projects: Vec<(String, String)>, // (ID, Name)
    pub selected_project_idx: usize,
}
