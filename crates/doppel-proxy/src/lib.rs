//! HTTP proxying: resolution, fault injection, upstream forwarding.

pub mod fault;
pub mod mock;
pub mod resolve;
pub mod rewrite;
pub mod server;
pub mod upstream;

pub use fault::{FaultDecision, OsSampler, Sampler, SequenceSampler, decide};
pub use mock::match_mock;
pub use resolve::resolve;
pub use server::{ProxyState, request_id, router, serve};
pub use upstream::{HOP_BY_HOP, UpstreamOutcome, error_response, forward, join_upstream};
