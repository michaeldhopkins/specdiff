fn main() {
    std::process::Command::new("git").status().ok();
    vcs_runner::run_jj(std::path::Path::new("."), &["log"]).ok();
}

#[cfg(test)]
mod tests {
    #[test]
    fn q_quits() {}
}
