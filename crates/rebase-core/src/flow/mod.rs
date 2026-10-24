//! Flows: multi-step agent workflows defined as a state machine.
pub mod definition;
pub mod starter;
pub mod template;

pub use definition::{
    approval_outputs, outputs_of, validate_flow, AgentState, ApprovalState, Flow, FlowInput,
    FlowState, Guard, GuardOp, OutputField, OutputType, OutputValue, Transition, Validation,
    VisitResult, FAILED, SUCCEEDED,
};
pub use starter::starter_flows;
pub use template::{list_bindings, render_template, resolve_inputs, BindingContext};
