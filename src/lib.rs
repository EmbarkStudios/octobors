// BEGIN - Embark standard lints v0.4
// do not change or add/remove here, but one can add exceptions after this section
// for more info see: <https://github.com/EmbarkStudios/rust-ecosystem/issues/59>
#![deny(unsafe_code)]
#![warn(
    clippy::all,
    clippy::await_holding_lock,
    clippy::char_lit_as_u8,
    clippy::checked_conversions,
    clippy::dbg_macro,
    clippy::debug_assert_with_mut_call,
    clippy::doc_markdown,
    clippy::empty_enum,
    clippy::enum_glob_use,
    clippy::exit,
    clippy::expl_impl_clone_on_copy,
    clippy::explicit_deref_methods,
    clippy::explicit_into_iter_loop,
    clippy::fallible_impl_from,
    clippy::filter_map_next,
    clippy::float_cmp_const,
    clippy::fn_params_excessive_bools,
    clippy::if_let_mutex,
    clippy::implicit_clone,
    clippy::imprecise_flops,
    clippy::inefficient_to_string,
    clippy::invalid_upcast_comparisons,
    clippy::large_types_passed_by_value,
    clippy::let_unit_value,
    clippy::linkedlist,
    clippy::lossy_float_literal,
    clippy::macro_use_imports,
    clippy::manual_ok_or,
    clippy::map_err_ignore,
    clippy::map_flatten,
    clippy::map_unwrap_or,
    clippy::match_on_vec_items,
    clippy::match_same_arms,
    clippy::match_wildcard_for_single_variants,
    clippy::mem_forget,
    clippy::mismatched_target_os,
    clippy::mut_mut,
    clippy::mutex_integer,
    clippy::needless_borrow,
    clippy::needless_continue,
    clippy::option_option,
    clippy::path_buf_push_overwrite,
    clippy::ptr_as_ptr,
    clippy::ref_option_ref,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::same_functions_in_if_condition,
    clippy::semicolon_if_nothing_returned,
    clippy::string_add_assign,
    clippy::string_add,
    clippy::string_lit_as_bytes,
    clippy::string_to_string,
    clippy::todo,
    clippy::trait_duplication_in_bounds,
    clippy::unimplemented,
    clippy::unnested_or_patterns,
    clippy::unused_self,
    clippy::useless_transmute,
    clippy::verbose_file_reads,
    clippy::zero_sized_map_values,
    future_incompatible,
    nonstandard_style,
    rust_2018_idioms
)]
// END - Embark standard lints v0.4

// See https://github.com/rust-lang/rust/issues/87417
#![allow(ungated_async_fn_track_caller)]

pub mod context;
mod merge;
pub mod process;
mod review;

use anyhow::{Context, Result};
use log::Instrument;
use process::{Actions, Analyzer, Pr};
use std::path::Path;
use tracing::{self as log, Level};

pub struct Octobors {
    pub config: context::Config,
    pub client: context::Client,
}

impl Octobors {
    pub fn new(path: &Path) -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN")
            .context("failed to read GITHUB_TOKEN environment variable")?
            .trim()
            .to_string();
        let contents = std::fs::read_to_string(path)?;
        let config: context::Config = toml::from_str(contents.as_str())?;
        let client = context::Client::new(
            token,
            config.owner.clone(),
            config.github_api_base.as_deref(),
            config.extra_headers.as_slice(),
        )?;

        Ok(Self { config, client })
    }

    pub async fn process_all(&self) -> Result<()> {
        for repo in self.config.repos.iter() {
            let span = log::span!(Level::INFO, "repo", name = repo.name.as_str());

            RepoProcessor::new(&self.config, &self.client, repo)
                .process()
                .instrument(span)
                .await?;
        }
        Ok(())
    }
}

pub struct RepoProcessor<'a> {
    pub config: &'a context::Config,
    pub client: &'a context::Client,
    pub repo_config: &'a context::RepoConfig,
}

impl<'a> RepoProcessor<'a> {
    pub fn new(
        config: &'a context::Config,
        client: &'a context::Client,
        repo_config: &'a context::RepoConfig,
    ) -> Self {
        Self {
            config,
            client,
            repo_config,
        }
    }

    pub async fn process(&self) -> Result<()> {
        let futures = self
            .client
            .get_pull_requests(&self.repo_config.name)
            .await?
            .into_iter()
            .map(|pr| {
                let span = log::span!(Level::INFO, "pr", number = pr.number);
                self.process_pr(pr).instrument(span)
            });
        futures::future::try_join_all(futures).await?;
        Ok(())
    }

    async fn process_pr(&self, pr: octocrab::models::pulls::PullRequest) -> Result<()> {
        let pr = Pr::from_octocrab_pull_request(pr);

        let actions = Analyzer::new(&pr, self.client, self.repo_config)
            .await?
            .required_actions()
            .await?;

        if self.config.dry_run {
            log::info!("dry-run {:?}", actions);
        } else {
            log::info!("applying {:?}", actions);
            self.apply(actions, &pr).await?;
        }

        Ok(())
    }

    pub async fn apply(&self, actions: Actions, pr: &Pr) -> Result<()> {
        let mut labels = pr.labels.iter().cloned().collect();
        let client = &self.client;
        let num = pr.number;

        process::remove_labels(
            client,
            &self.repo_config.name,
            num,
            &mut labels,
            actions.remove_labels.into_iter(),
        )
        .await?;

        process::add_labels(
            client,
            &self.repo_config.name,
            num,
            &mut labels,
            actions.add_labels.into_iter(),
        )
        .await?;

        for comment in actions.post_comment {
            log::debug!("Posting a comment: {comment}");
            process::post_comment(client, &self.repo_config.name, num, comment).await?;
        }

        if actions.merge {
            log::info!("Attempting to merge");
            merge::queue(self.client, pr, self.repo_config).await?;
        }
        Ok(())
    }
}
