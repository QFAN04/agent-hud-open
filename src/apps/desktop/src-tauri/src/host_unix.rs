/// Host options on Unix desktop builds (`None` keeps the canonical owner delegated).
pub(crate) fn options() -> agent_hud_host::HostOptions {
    agent_hud_host::HostOptions::default()
}
