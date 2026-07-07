-- Project-local Neovim config (sourced via `exrc`; run `:trust` to enable).
--
-- Goal: keep rust-analyzer — and the cargo commands it generates for Neotest
-- runnables — on default features only, so the release-only `builtin_migrations`
-- feature isn't compiled during analysis/tests. Its
-- `include_dir!("$CARGO_WORKSPACE_DIR/migrations")` macro panics at compile time
-- with `MissingVariable { "CARGO_WORKSPACE_DIR" }` (see
-- src/database/migrator/builtin_migrations.rs). Enable it only at release time:
--   cargo build --release --features builtin_migrations
--
-- Why not `vim.g.rustaceanvim`: rustaceanvim snapshots `vim.g.rustaceanvim` once,
-- at first require of its config module. The Neotest adapter
-- (`require("rustaceanvim.neotest")` in the global neotest spec) force-loads
-- rustaceanvim at startup, *before* exrc runs, freezing the snapshot with
-- LazyVim's `cargo.allFeatures = true`. A later `vim.g` change is ignored.
--
-- Instead we set the native `vim.lsp.config['rust-analyzer']`, which
-- rustaceanvim's `M.start` re-reads on every client start and force-merges over
-- its snapshot (lua/rustaceanvim/lsp/init.lua) — so this wins regardless of the
-- snapshot, and applies at client init (no didChangeConfiguration needed).
--
-- NOTE: `vim.lsp.config` is global to the rust-analyzer server for the session.
-- If you open an unrelated Rust project in the *same* Neovim session, it would
-- also get default features. exrc sessions are effectively per-project, so this
-- is acceptable; just relaunch Neovim for a different project.

vim.lsp.config("rust-analyzer", {
  settings = { ["rust-analyzer"] = { cargo = { allFeatures = false } } },
})

-- Handle the race where rust-analyzer already started before exrc ran (e.g. a
-- `.rs` file passed on the command line): restart so it re-reads the config
-- above. Scoped to clients rooted in this project.
local PROJECT_ROOT = vim.fs.normalize(vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h"))
for _, client in ipairs(vim.lsp.get_clients({ name = "rust-analyzer" })) do
  local root = client.config.root_dir and vim.fs.normalize(client.config.root_dir)
  if root and (root == PROJECT_ROOT or vim.startswith(root, PROJECT_ROOT .. "/")) then
    vim.schedule(function()
      vim.cmd("RustAnalyzer restart")
    end)
    break
  end
end
