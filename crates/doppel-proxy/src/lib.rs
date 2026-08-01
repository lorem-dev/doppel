//! HTTP proxying: resolution, fault injection, upstream forwarding.

pub mod fault;
pub mod resolve;
pub mod upstream;

pub use fault::{FaultDecision, OsSampler, Sampler, SequenceSampler, decide};
pub use resolve::resolve;
pub use upstream::{HOP_BY_HOP, UpstreamOutcome, error_response, forward, join_upstream};
