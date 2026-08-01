//! HTTP proxying: resolution, fault injection, upstream forwarding.

pub mod fault;

pub use fault::{FaultDecision, OsSampler, Sampler, SequenceSampler, decide};
