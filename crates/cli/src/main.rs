fn main() {
    // Behave like a classic Unix tool when piped into `head` etc.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    shellmind::cli::run();
}
