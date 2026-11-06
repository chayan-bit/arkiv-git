//! `arkiv-git-mcp`: the arkiv-git MCP server on stdio (docs/p3/mcp.md).

use arkiv_git_mcp::ArkivGit;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() {
    let server = match ArkivGit::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("arkiv-git-mcp: {e}");
            std::process::exit(2);
        }
    };
    let served = match server.serve(rmcp::transport::stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("arkiv-git-mcp: {e}");
            std::process::exit(3);
        }
    };
    if let Err(e) = served.waiting().await {
        eprintln!("arkiv-git-mcp: {e}");
        std::process::exit(3);
    }
}
