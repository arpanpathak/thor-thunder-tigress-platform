//! `thor`: serves a model on the Jetson Thor with the engine a person chooses.
//!
//! A person picks an engine (TensorRT Edge-LLM, llama.cpp or vLLM), then a model from disk
//! or from a live Hugging Face search, then a context. `thor` estimates the memory from the
//! model's own `config.json` and stops before any change when the model does not fit.
//! Otherwise it downloads the model, backs up and writes `~/.config/thor-chat/env`,
//! restarts the engine's systemd user service, and waits until the server answers. Every
//! question has a flag for scripts.

pub mod catalog;
pub mod cli;
pub mod commands;
pub mod docker;
pub mod edge;
pub mod engine;
pub mod error;
pub mod hub;
pub mod machine;
pub mod memory;
pub mod plan;
pub mod recipes;
pub mod serve;
pub mod service;
pub mod settings;
pub mod start;
pub mod web;
