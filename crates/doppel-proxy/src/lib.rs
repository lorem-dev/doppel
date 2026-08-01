//! HTTP proxying: resolution, fault injection, upstream forwarding.

pub mod fault;
pub mod resolve;

pub use fault::{FaultDecision, OsSampler, Sampler, SequenceSampler, decide};
pub use resolve::resolve;
