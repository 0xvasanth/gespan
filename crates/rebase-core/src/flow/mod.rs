//! Flows: multi-step agent workflows defined as a state machine.
pub mod contract;
pub mod definition;
pub mod run;
pub mod starter;
pub mod store;
pub mod template;

pub use definition::{
    approval_outputs, outputs_of, validate_flow, AgentState, ApprovalState, Flow, FlowInput,
    FlowState, Guard, GuardOp, OutputField, OutputType, OutputValue, Transition, Validation,
    VisitResult, FAILED, SUCCEEDED,
};
pub use run::{FlowRun, FlowVisit, RunStatus, VisitStatus};
pub use starter::starter_flows;
pub use store::FlowStore;
pub use template::{list_bindings, render_template, resolve_inputs, BindingContext};
