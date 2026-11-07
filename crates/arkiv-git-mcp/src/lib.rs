//! arkiv-git-mcp: an MCP server that lets an AI agent work arkiv-git repos (docs/p3/mcp.md).
//!
//! Every tool is one `arkiv-git` or `git` invocation; the CLI keeps all signing, fetching,
//! program and escrow logic, and its `E_*` error codes come back in the tool error as
//! `{code, message, exit, stdout}`. Keys never pass through tool arguments: the children
//! use the identity under `$ARKIV_GIT_HOME` and the backend from `$ARKIV_GIT_BACKEND`.

pub mod run;

use std::path::PathBuf;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::Value;

pub use run::Runner;

type ToolResult = Result<String, String>;

const INSTRUCTIONS: &str = "Git repositories on Arkiv. A repo is named by its url \
`arkiv::<rid>` (or the bare rid). Find work with op_list (issues with a `bounty` that no \
patch names yet), clone with repo_clone, edit files in the clone, commit and push them with \
git_push, then attach_proof to post the patch with its receipt. program_status shows \
what the repo's pinned program decided; escrow_status audits payouts. Errors carry the \
stable arkiv-git code (E_*).";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RepoArg {
    /// Repo url `arkiv::<rid>` or bare rid.
    pub repo: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloneArgs {
    /// Repo url `arkiv::<rid>` or bare rid.
    pub repo: String,
    /// Directory to clone into; must not exist yet.
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Repo url `arkiv::<rid>` or bare rid.
    pub repo: String,
    /// Only ops of this type, e.g. `xyz.arkiv.issue`.
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    /// Only ops with this action, e.g. `create`.
    pub action: Option<String>,
    /// Only ops of this object (64-char hex id).
    pub object: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PostArgs {
    /// Repo url `arkiv::<rid>` or bare rid.
    pub repo: String,
    /// Object type, e.g. `xyz.arkiv.comment`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// `create` starts a new object; any other action needs `object`.
    pub action: String,
    /// The object this op belongs to (64-char hex id), for actions other than `create`.
    pub object: Option<String>,
    /// Ids of earlier ops of the same object this op saw.
    #[serde(default)]
    pub parents: Vec<String>,
    /// Payload: a JSON object with no floats.
    pub payload: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachArgs {
    /// Repo url `arkiv::<rid>` or bare rid.
    pub repo: String,
    /// Op type; default `xyz.arkiv.patch`.
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    /// Op action; default `create`.
    pub action: Option<String>,
    /// The object, for actions other than `create`.
    pub object: Option<String>,
    /// Ids of earlier ops of the same object.
    #[serde(default)]
    pub parents: Vec<String>,
    /// Payload object without `proofs`, e.g. `{"title": .., "head": <commit>, "issue": <id>}`.
    pub payload: Value,
    /// Path of the RISC Zero receipt file.
    pub receipt: PathBuf,
    /// Path of the receipt's public statement JSON.
    pub public: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PushArgs {
    /// Repo url `arkiv::<rid>` or bare rid.
    pub repo: String,
    /// The local clone (from repo_clone).
    pub dir: PathBuf,
    /// Branch to push `HEAD` to, e.g. `agent/1a2b3c4d`; it lands in this identity's namespace.
    pub branch: String,
    /// If set, first commit every change in `dir` with this message.
    pub message: Option<String>,
}

/// The arkiv-git MCP server.
#[derive(Debug, Clone)]
pub struct ArkivGit {
    runner: Runner,
    tool_router: ToolRouter<Self>,
}

fn url(repo: &str) -> String {
    if repo.starts_with("arkiv::") {
        repo.to_owned()
    } else {
        format!("arkiv::{repo}")
    }
}

fn err(v: Value) -> String {
    v.to_string()
}

fn op_args(cmd: &str, repo: &str, ty: &str, action: &str, object: &Option<String>) -> Vec<String> {
    let mut a: Vec<String> = ["op", cmd, "--repo", repo, "--type", ty, "--action", action]
        .map(String::from)
        .into();
    if let Some(o) = object {
        a.extend(["--object".into(), o.clone()]);
    }
    a
}

fn with_payload(mut a: Vec<String>, parents: &[String], payload: &Value) -> Vec<String> {
    for p in parents {
        a.extend(["--parent".into(), p.clone()]);
    }
    a.extend(["--payload-json".into(), payload.to_string()]);
    a
}

/// `{"id": <last stdout line>}`: `op post` and `op attach-proof` print the op id last.
fn id_json(out: &str) -> String {
    let id = out.lines().last().unwrap_or_default().trim();
    serde_json::json!({ "id": id }).to_string()
}

#[tool_router]
impl ArkivGit {
    pub fn new(runner: Runner) -> Self {
        Self {
            runner,
            tool_router: Self::tool_router(),
        }
    }

    /// A server using [`Runner::from_env`].
    pub fn from_env() -> Result<Self, String> {
        Ok(Self::new(Runner::from_env()))
    }

    #[tool(description = "Clone an arkiv-git repo into a new local directory with git.")]
    pub async fn repo_clone(&self, Parameters(a): Parameters<CloneArgs>) -> ToolResult {
        let parent = a.dir.parent().filter(|p| !p.as_os_str().is_empty());
        let cwd = parent.map_or_else(|| PathBuf::from("."), PathBuf::from);
        let args = ["clone", "-q", &url(&a.repo), &a.dir.to_string_lossy()].map(String::from);
        self.runner.git(&cwd, &args).await.map_err(err)?;
        Ok(serde_json::json!({ "dir": a.dir }).to_string())
    }

    #[tool(
        description = "Optionally commit all changes in a clone, then git push HEAD to \
        refs/heads/<branch> of the repo under this identity; returns {head} (the full commit sha)."
    )]
    pub async fn git_push(&self, Parameters(a): Parameters<PushArgs>) -> ToolResult {
        let git = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            let dir = a.dir.clone();
            async move { self.runner.git(&dir, &args).await.map_err(err) }
        };
        if let Some(m) = &a.message {
            git(&["add", "-A"]).await?;
            git(&["commit", "-q", "-m", m]).await?;
        }
        let refspec = format!("HEAD:refs/heads/{}", a.branch);
        git(&["push", "-q", &url(&a.repo), &refspec]).await?;
        let head = git(&["rev-parse", "HEAD"]).await?;
        Ok(serde_json::json!({ "head": head.trim() }).to_string())
    }

    #[tool(
        description = "List a repo's verified collaborative-object ops (issues, patches, reviews, \
        agent grants, comments, payments) in Arkiv order as a JSON array of {id, object, author, \
        type, action, parents, ts_ms, block, payload}. Filter by type, action and object."
    )]
    pub async fn op_list(&self, Parameters(a): Parameters<ListArgs>) -> ToolResult {
        let mut args: Vec<String> = ["op", "list", "--json", "--repo", &url(&a.repo)]
            .map(String::from)
            .into();
        if let Some(t) = &a.type_name {
            args.extend(["--type".into(), t.clone()]);
        }
        if let Some(o) = &a.object {
            args.extend(["--object".into(), o.clone()]);
        }
        let out = self.runner.arkiv(&args).await.map_err(err)?;
        let ops: Vec<Value> = serde_json::from_str(&out).map_err(|e| e.to_string())?;
        let keep = |o: &Value| a.action.as_deref().is_none_or(|x| o["action"] == x);
        Ok(Value::Array(ops.into_iter().filter(keep).collect()).to_string())
    }

    #[tool(description = "Sign and post one op as this agent's identity; returns {id}.")]
    pub async fn op_post(&self, Parameters(a): Parameters<PostArgs>) -> ToolResult {
        let base = op_args("post", &url(&a.repo), &a.type_name, &a.action, &a.object);
        let out = self
            .runner
            .arkiv(&with_payload(base, &a.parents, &a.payload))
            .await
            .map_err(err)?;
        Ok(id_json(&out))
    }

    #[tool(
        description = "Post a RISC Zero receipt as xyz.arkiv.blob ops (chunks already on the \
        repo are skipped), then post one op (default: xyz.arkiv.patch create) whose payload \
        gets a `proofs` entry citing them; returns {id} of that op."
    )]
    pub async fn attach_proof(&self, Parameters(a): Parameters<AttachArgs>) -> ToolResult {
        let ty = a.type_name.as_deref().unwrap_or("xyz.arkiv.patch");
        let action = a.action.as_deref().unwrap_or("create");
        let base = op_args("attach-proof", &url(&a.repo), ty, action, &a.object);
        let mut args = with_payload(base, &a.parents, &a.payload);
        args.extend(["--receipt".into(), a.receipt.to_string_lossy().into()]);
        args.extend(["--public".into(), a.public.to_string_lossy().into()]);
        let out = self.runner.arkiv(&args).await.map_err(err)?;
        Ok(id_json(&out))
    }

    #[tool(
        description = "Fold the repo's pinned program: its state (bounties, patches, grants, \
        rejected ops with reasons), accepted heads, rejected pushes and obligations."
    )]
    pub async fn program_status(&self, Parameters(a): Parameters<RepoArg>) -> ToolResult {
        let args = ["program", "status", "--repo", &url(&a.repo)].map(String::from);
        self.runner.arkiv(&args).await.map_err(err)
    }

    #[tool(
        description = "Keyless escrow audit of the repo: deposits, settlements, balance owed \
        and FLAG lines; a flagged ledger is a tool error with code `flagged`."
    )]
    pub async fn escrow_status(&self, Parameters(a): Parameters<RepoArg>) -> ToolResult {
        let args = ["escrow", "audit", "--repo", &url(&a.repo)].map(String::from);
        self.runner.arkiv(&args).await.map_err(err)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ArkivGit {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("arkiv-git", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests;
