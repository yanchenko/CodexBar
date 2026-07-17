//! AgentBar CLI binary (`agentbar`). Scaffold prints version only.

fn main() {
    println!("agentbar {}", env!("CARGO_PKG_VERSION"));
}
