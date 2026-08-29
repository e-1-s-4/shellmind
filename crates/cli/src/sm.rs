//! The short-form `sm` binary. It is the same program as `shellmind`;
//! both names exist purely for ergonomics (`sm explain ...`).

fn main() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    shellmind::cli::run();
}
