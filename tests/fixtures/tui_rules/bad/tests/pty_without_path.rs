use portable_pty::CommandBuilder;

fn spawn() -> CommandBuilder {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_specdiff"));
    cmd.env_clear();
    cmd
}
