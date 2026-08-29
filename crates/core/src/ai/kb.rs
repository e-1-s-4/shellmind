//! Static knowledge base powering the offline engine.
//!
//! Three tables:
//!
//! * [`ERROR_PATTERNS`]  – stderr patterns → concrete fixes,
//! * [`INTENTS`]         – natural language → command templates,
//! * [`EXPLAIN_KB`]      – binary summaries with common examples.
//!
//! All entries are pure data; matching logic lives in `offline.rs`.

/// A fix for a common failure.
pub struct ErrorPattern {
    pub id: &'static str,
    /// Regex matched (case-insensitively) against the captured error text.
    pub regex: &'static str,
    /// Fix template; may contain `{branch}` / `{port}` / `{name}` /
    /// `{module}` placeholders filled by the matcher.
    pub fixes: &'static [(&'static str, &'static str)],
    /// Human explanation shown alongside the first fix.
    pub explanation: &'static str,
}

/// A natural-language intent.
pub struct Intent {
    pub id: &'static str,
    /// All keyword sets that trigger this intent (lowercase tokens).
    pub patterns: &'static [&'static [&'static str]],
    /// Command template, may contain `{size}`, `{port}`, `{branch}`,
    /// `{file}`, `{dir}` placeholders extracted from the query.
    pub command: &'static str,
    pub explanation: &'static str,
    /// Safer alternatives for destructive intents.
    pub safer: &'static [&'static str],
    /// When true the command differs on macOS (`macos` field is used).
    pub os_aware: bool,
    /// macOS variant (only when `os_aware`).
    pub macos: &'static str,
}

/// A man-page-lite summary for `sm explain`.
pub struct ExplainEntry {
    pub binary: &'static str,
    pub summary: &'static str,
    /// (label, example command) pairs.
    pub examples: &'static [(&'static str, &'static str)],
}

pub static ERROR_PATTERNS: &[ErrorPattern] = &[
    ErrorPattern {
        id: "git_no_upstream",
        regex: r"(?i)(has no upstream branch|no upstream branch configured|The current branch \S+ has no upstream)",
        fixes: &[
            ("git push --set-upstream origin {branch}", "Your local {branch} branch is not tracking a remote branch. This pushes {branch} and sets it to track origin/{branch}."),
        ],
        explanation: "The local branch has no upstream tracking branch on the remote.",
    },
    ErrorPattern {
        id: "git_not_a_repo",
        regex: r"(?i)(not a git repository|does not have a commit checked out)",
        fixes: &[
            ("git init", "This directory is not inside a git working tree; initialize a repository here."),
            ("git clone <url>", "If you meant to work on an existing project, clone it instead."),
        ],
        explanation: "The current directory is not inside a git repository.",
    },
    ErrorPattern {
        id: "git_index_lock",
        regex: r"(?i)(another git process seems to be running|\.git/index\.lock)",
        fixes: &[
            ("rm -f .git/index.lock", "A previous git process left a stale lock file. Only remove it when no git process is running."),
        ],
        explanation: "A stale .git/index.lock is blocking git operations.",
    },
    ErrorPattern {
        id: "git_refspec",
        regex: r"(?i)(src refspec \S+ does not match any)",
        fixes: &[
            ("git push -u origin HEAD", "The branch you tried to push does not exist yet locally; push the current branch under its own name."),
        ],
        explanation: "The ref you tried to push does not exist.",
    },
    ErrorPattern {
        id: "command_not_found",
        regex: r"(?i)(command not found|not found.*is not.*command|zsh: command not found|bash: \S+: command not found|no such file or directory.*\/usr\/bin)",
        fixes: &[],
        explanation: "The command is not installed (or misspelled).",
    },
    ErrorPattern {
        id: "module_not_found",
        regex: r"(?i)ModuleNotFoundError: No module named '([^']+)'",
        fixes: &[
            ("source .venv/bin/activate", "Your Python virtual environment is not active — the module is probably installed inside it."),
            ("pip install -r requirements.txt", "Or install the project's dependencies into the active environment."),
            ("pip install {module}", "Install just the missing module."),
        ],
        explanation: "A Python module is missing from the active interpreter.",
    },
    ErrorPattern {
        id: "port_in_use",
        regex: r"(?i)(Address already in use|port is already allocated|EADDRINUSE|address.*in use)",
        fixes: &[
            ("lsof -ti :{port} | xargs kill -9", "Another process is bound to port {port}; this finds and kills it (check with `lsof -i :{port}` first)."),
            ("lsof -i :{port}", "Inspect what is listening on the port before killing anything."),
        ],
        explanation: "A port needed by the process is already taken.",
    },
    ErrorPattern {
        id: "permission_denied",
        regex: r"(?i)(permission denied|EACCES|access denied)",
        fixes: &[
            ("sudo {command}", "Insufficient permissions — retry with elevation. Double-check the command first."),
            ("chmod +x {file}", "If it is a script you own, it may simply lack the executable bit."),
        ],
        explanation: "The operation was blocked by file or socket permissions.",
    },
    ErrorPattern {
        id: "docker_daemon_perms",
        regex: r"(?i)(permission denied while trying to connect to the Docker daemon socket|Cannot connect to the Docker daemon)",
        fixes: &[
            ("sudo systemctl start docker", "The Docker daemon may not be running."),
            ("sudo usermod -aG docker $USER", "Or your user lacks docker group membership (log out and back in afterwards)."),
        ],
        explanation: "The Docker CLI cannot talk to the daemon.",
    },
    ErrorPattern {
        id: "kubectl_connection",
        regex: r"(?i)(The connection to the server localhost:8080 was refused|connection refused|Unable to connect to the server)",
        fixes: &[
            ("kubectl config current-context", "kubectl has no usable cluster configuration; check which context is active."),
            ("kubectl cluster-info", "Verify the control plane is reachable."),
        ],
        explanation: "kubectl cannot reach the cluster API server.",
    },
    ErrorPattern {
        id: "npm_missing_script",
        regex: r#"(?i)(Missing script: "?[A-Za-z0-9:_.-]+"?)"#,
        fixes: &[
            ("npm run", "The script does not exist in package.json; this lists the ones that do."),
        ],
        explanation: "npm could not find that script in package.json.",
    },
    ErrorPattern {
        id: "npm_eacces",
        regex: r"(?i)(EACCES: permission denied)",
        fixes: &[
            ("npm cache clean --force", "A corrupted cache entry is often the culprit."),
            ("sudo npm install -g", "Global installs may need elevation (prefer fixing npm's prefix instead)."),
        ],
        explanation: "npm hit a filesystem permission error.",
    },
    ErrorPattern {
        id: "syntax_error_token",
        regex: r"(?i)(syntax error near unexpected token|unexpected token)",
        fixes: &[
            ("# Quote the argument: <cmd> \"<arg>\"", "The shell parsed your command as syntax — most likely an unquoted (, ) or | character. Wrap the argument in quotes."),
        ],
        explanation: "The shell could not parse the command line.",
    },
    ErrorPattern {
        id: "ssh_host_key",
        regex: r"(?i)(Host key verification failed|REMOTE HOST IDENTIFICATION HAS CHANGED)",
        fixes: &[
            ("ssh -o StrictHostKeyChecking=accept-new <host>", "Accept new host keys automatically the first time you connect."),
            ("ssh-keygen -R <host>", "If the server legitimately changed its key, remove the stale entry from known_hosts."),
        ],
        explanation: "SSH refused the connection over a host key mismatch.",
    },
    ErrorPattern {
        id: "disk_full",
        regex: r"(?i)(No space left on device|ENOSPC)",
        fixes: &[
            ("df -h", "Check which filesystem is full."),
            ("docker system df", "Docker images/logs are a frequent cause — see how much they hold."),
        ],
        explanation: "The filesystem ran out of space.",
    },
    ErrorPattern {
        id: "file_not_found",
        regex: r"(?i)(No such file or directory)",
        fixes: &[],
        explanation: "A path in the command does not exist.",
    },
    ErrorPattern {
        id: "merge_conflict",
        regex: r"(?i)(CONFLICT \(|Automatic merge failed|fix conflicts)",
        fixes: &[
            ("git status", "List the conflicced files."),
            ("git diff --name-only --diff-filter=U", "Just the unresolved paths."),
            ("git merge --abort", "Or walk away from the merge entirely."),
        ],
        explanation: "A merge/rebase hit unresolved conflicts.",
    },
    ErrorPattern {
        id: "divergent_branches",
        regex: r"(?i)(divergent branches|non-fast-forward|fetch first)",
        fixes: &[
            ("git pull --rebase", "Replay your local commits on top of the remote branch."),
            ("git pull --no-rebase", "Or merge the remote changes into yours."),
        ],
        explanation: "Local and remote branches have diverged.",
    },
];

pub static INTENTS: &[Intent] = &[
    // ---------------- files & disk ----------------
    Intent {
        id: "find_large_files",
        patterns: &[
            &["find", "large", "files"],
            &["find", "files", "larger"],
            &["largest", "files"],
            &["big", "files"],
            &["files", "size"],
        ],
        command: "find . -type f -size +{size} -exec ls -lh {} + | sort -k5 -h",
        explanation: "Searches the current directory recursively for regular files larger than {size} and lists them sorted by size.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "delete_large_files",
        patterns: &[
            &["find", "large", "files", "delete"],
            &["delete", "files", "larger"],
            &["remove", "large", "files"],
            &["find", "files", "larger", "delete"],
        ],
        command: "find . -type f -size +{size} -delete",
        explanation: "Recursively deletes regular files larger than {size}. Double-check the size and directory before running.",
        safer: &[
            "find . -type f -size +{size} -print",
            "find . -type f -size +{size} -exec rm -i {} +",
        ],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "disk_usage_by_folder",
        patterns: &[
            &["disk", "usage", "folder"],
            &["disk", "usage", "directory"],
            &["folder", "sizes"],
            &["size", "folders"],
            &["du", "folder"],
            &["disk", "usage"],
        ],
        command: "du -h --max-depth=1 | sort -hr",
        explanation: "Shows how much disk each immediate subdirectory uses, largest first.",
        safer: &[],
        os_aware: true,
        macos: "du -h -d 1 | sort -hr",
    },
    Intent {
        id: "free_disk_space",
        patterns: &[&["free", "disk"], &["disk", "space"], &["how", "much", "space"], &["df"]],
        command: "df -h",
        explanation: "Reports free and used space for every mounted filesystem.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "count_files",
        patterns: &[&["count", "files"], &["how", "many", "files"]],
        command: "find . -type f | wc -l",
        explanation: "Counts every regular file under the current directory.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "find_file_by_name",
        patterns: &[&["find", "file"], &["find", "name"], &["locate", "file"], &["search", "file", "name"]],
        command: "find . -iname '*{file}*'",
        explanation: "Case-insensitive search for files whose name contains the text.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "search_file_contents",
        patterns: &[&["search", "content"], &["grep", "recursive"], &["search", "text", "files"], &["find", "text", "files"]],
        command: "grep -rn --color=auto '{file}' .",
        explanation: "Recursively searches all files for the text, showing file and line numbers.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    // ---------------- archives ----------------
    Intent {
        id: "compress_folder",
        patterns: &[
            &["compress", "folder"],
            &["compress", "directory"],
            &["tar", "folder"],
            &["archive", "folder"],
            &["create", "tar"],
            &["zip", "folder"],
        ],
        command: "tar -czvf {file}.tar.gz {dir}",
        explanation: "Creates a gzip-compressed tarball of the directory.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "extract_tar_gz",
        patterns: &[
            &["extract", "tar"],
            &["untar"],
            &["uncompress", "tar"],
            &["extract", "archive"],
            &["decompress", "tar"],
            &["extract", "gz"],
        ],
        command: "tar -xzvf {file}",
        explanation: "Extracts a .tar.gz archive into the current directory.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "list_archive",
        patterns: &[&["list", "archive"], &["tar", "contents"], &["inspect", "tar"], &["preview", "archive"]],
        command: "tar -tzvf {file}",
        explanation: "Lists the contents of a tarball without extracting it.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    // ---------------- docker ----------------
    Intent {
        id: "docker_remove_unused_images",
        patterns: &[
            &["docker", "remove", "images"],
            &["docker", "remove", "unused"],
            &["docker", "prune"],
            &["remove", "dangling", "images"],
            &["docker", "clean", "images"],
            &["delete", "unused", "images"],
        ],
        command: "docker image prune -a",
        explanation: "Removes ALL images not currently used by a container — not just dangling ones.",
        safer: &[
            "docker image prune",
            "docker image prune --filter until=24h",
        ],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "docker_show_containers",
        patterns: &[
            &["show", "containers"],
            &["running", "containers"],
            &["list", "containers"],
            &["docker", "ps"],
        ],
        command: "docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}'",
        explanation: "Lists running containers with name, status and published ports.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "docker_stop_all",
        patterns: &[&["docker", "stop", "all"], &["stop", "containers"]],
        command: "docker stop $(docker ps -q)",
        explanation: "Stops every running container.",
        safer: &["docker ps", "docker stop <container>"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "docker_logs_tail",
        patterns: &[&["docker", "logs"], &["container", "logs"], &["tail", "logs", "container"]],
        command: "docker logs -f --tail 100 {file}",
        explanation: "Follows the last 100 log lines of a container (Ctrl+C to stop).",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    // ---------------- kubernetes ----------------
    Intent {
        id: "k8s_get_pods",
        patterns: &[
            &["kubernetes", "pods"],
            &["kubectl", "pods"],
            &["list", "pods"],
            &["get", "pods"],
            &["show", "pods"],
        ],
        command: "kubectl get pods -n {namespace}",
        explanation: "Lists pods in the given namespace (or your current one).",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "k8s_logs",
        patterns: &[&["kubectl", "logs"], &["kubernetes", "logs"], &["pod", "logs"], &["tail", "pod"]],
        command: "kubectl logs -f --tail=100 {file}",
        explanation: "Follows the logs of a pod.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "k8s_restart_deployment",
        patterns: &[
            &["kubectl", "restart"],
            &["restart", "deployment"],
            &["rolling", "restart"],
            &["kubernetes", "restart"],
        ],
        command: "kubectl rollout restart deployment/{file}",
        explanation: "Performs a rolling restart of a deployment.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "k8s_switch_namespace",
        patterns: &[&["kubectl", "namespace"], &["switch", "namespace"], &["change", "namespace"]],
        command: "kubectl config set-context --current --namespace={namespace}",
        explanation: "Sets the default namespace of the current kube context.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    // ---------------- git ----------------
    Intent {
        id: "git_undo_last_commit",
        patterns: &[
            &["undo", "commit"],
            &["undo", "last", "commit"],
            &["revert", "last", "commit"],
            &["uncommit"],
        ],
        command: "git reset --soft HEAD~1",
        explanation: "Undoes the last commit but keeps its changes staged — nothing is lost.",
        safer: &["git reset --soft HEAD~1"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_discard_local_changes",
        patterns: &[
            &["discard", "changes"],
            &["undo", "changes"],
            &["reset", "local", "changes"],
            &["revert", "local", "changes"],
            &["discard", "local"],
        ],
        command: "git restore .",
        explanation: "Discards uncommitted changes to tracked files in the working tree.",
        safer: &[
            "git stash",
            "git diff  # review what you are about to lose",
        ],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_unstage",
        patterns: &[&["unstage"], &["undo", "add"], &["unstage", "files"]],
        command: "git restore --staged .",
        explanation: "Unstages everything without touching the working tree.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_create_branch",
        patterns: &[&["create", "branch"], &["new", "branch"], &["git", "branch"], &["switch", "branch"], &["checkout", "branch"]],
        command: "git switch -c {branch}",
        explanation: "Creates a new branch and switches to it.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_amend",
        patterns: &[&["amend"], &["edit", "last", "commit"], &["change", "commit", "message"]],
        command: "git commit --amend",
        explanation: "Re-opens the last commit for editing (add -m 'msg' to change the message).",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_delete_branch",
        patterns: &[&["delete", "branch"], &["remove", "branch"]],
        command: "git branch -d {branch}",
        explanation: "Deletes a merged local branch (-D forces deletion of unmerged branches).",
        safer: &["git branch --list", "git branch -d {branch}"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_stash_pop",
        patterns: &[&["stash", "pop"], &["apply", "stash"], &["git", "stash"]],
        command: "git stash pop",
        explanation: "Re-applies the most recent stash and removes it from the stash list.",
        safer: &["git stash list", "git stash apply"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "git_sync_main",
        patterns: &[&["update", "main"], &["pull", "latest"], &["sync", "main"], &["update", "master"]],
        command: "git checkout main && git pull --rebase && git checkout -",
        explanation: "Updates main with the latest remote commits and returns to your branch.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    // ---------------- processes & network ----------------
    Intent {
        id: "kill_port",
        patterns: &[&["kill", "port"], &["free", "port"], &["stop", "port"], &["process", "port"]],
        command: "lsof -ti :{port} | xargs kill -9",
        explanation: "Finds whatever listens on the port and force-kills it.",
        safer: &["lsof -i :{port}", "lsof -ti :{port} | xargs kill"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "list_open_ports",
        patterns: &[&["open", "ports"], &["listening", "ports"], &["list", "ports"], &["show", "ports"]],
        command: "ss -tulpn",
        explanation: "Lists all listening TCP/UDP sockets with owning processes.",
        safer: &[],
        os_aware: true,
        macos: "lsof -i -P -n | grep LISTEN",
    },
    Intent {
        id: "processes_by_cpu",
        patterns: &[&["processes", "cpu"], &["top", "cpu"], &["cpu", "usage"], &["memory", "usage"], &["top", "processes"]],
        command: "ps aux --sort=-%cpu | head -15",
        explanation: "Shows the 15 processes consuming the most CPU.",
        safer: &[],
        os_aware: true,
        macos: "ps aux -r | head -15",
    },
    // ---------------- node / python ----------------
    Intent {
        id: "npm_update_all",
        patterns: &[&["npm", "update"], &["update", "packages"], &["update", "dependencies"], &["upgrade", "packages"]],
        command: "npm update",
        explanation: "Updates packages to the newest versions allowed by package.json ranges.",
        safer: &["npm outdated"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "npm_list_outdated",
        patterns: &[&["outdated", "packages"], &["npm", "outdated"], &["old", "packages"]],
        command: "npm outdated",
        explanation: "Lists packages with newer versions available.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "pip_install_requirements",
        patterns: &[&["pip", "install"], &["install", "requirements"], &["python", "dependencies"]],
        command: "pip install -r requirements.txt",
        explanation: "Installs every dependency pinned in requirements.txt.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "venv_create",
        patterns: &[&["create", "venv"], &["virtual", "environment"], &["new", "venv"]],
        command: "python3 -m venv .venv && source .venv/bin/activate",
        explanation: "Creates and activates a fresh virtual environment in .venv/.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    // ---------------- misc utilities ----------------
    Intent {
        id: "serve_folder_http",
        patterns: &[&["serve", "folder"], &["http", "server"], &["static", "server"], &["serve", "directory"]],
        command: "python3 -m http.server 8000",
        explanation: "Serves the current directory over HTTP on port 8000.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "generate_ssh_key",
        patterns: &[&["ssh", "key"], &["generate", "key"], &["new", "ssh", "key"]],
        command: "ssh-keygen -t ed25519 -C \"$(whoami)@$(hostname)\"",
        explanation: "Generates a modern Ed25519 SSH key pair.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "copy_ssh_key",
        patterns: &[&["copy", "ssh", "key"], &["ssh", "copy", "id"]],
        command: "ssh-copy-id {user}@{host}",
        explanation: "Installs your public key on a remote host for passwordless login.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "rsync_folder",
        patterns: &[&["rsync"], &["sync", "folder"], &["copy", "folder", "remote"]],
        command: "rsync -avz --progress {dir}/ {host}:{dir}/",
        explanation: "Incrementally syncs a directory to a remote host over SSH.",
        safer: &["rsync -avzn --progress {dir}/ {host}:{dir}/  # dry run"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "create_symlink",
        patterns: &[&["symlink"], &["symbolic", "link"], &["ln"]],
        command: "ln -s {file} {dir}",
        explanation: "Creates a symbolic link pointing at the target path.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "list_env_vars",
        patterns: &[&["environment", "variables"], &["env", "vars"], &["show", "env"], &["list", "env"]],
        command: "printenv | sort",
        explanation: "Prints all environment variables, sorted.",
        safer: &[],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "restart_service",
        patterns: &[&["restart", "service"], &["systemd", "restart"], &["service", "restart"]],
        command: "sudo systemctl restart {file}",
        explanation: "Restarts a systemd service.",
        safer: &["systemctl status {file}"],
        os_aware: false,
        macos: "",
    },
    Intent {
        id: "follow_log_file",
        patterns: &[&["tail", "log"], &["follow", "log"], &["watch", "log"], &["tail", "file"]],
        command: "tail -f {file}",
        explanation: "Follows a log file as it grows (Ctrl+C to stop).",
        safer: &[],
        os_aware: false,
        macos: "",
    },
];

pub static EXPLAIN_KB: &[ExplainEntry] = &[
    ExplainEntry {
        binary: "tar",
        summary: "tar bundles files into a single archive, optionally compressed with gzip (-z), bzip2 (-j) or xz (-J).",
        examples: &[
            ("Create archive", "tar -czvf archive.tar.gz folder/"),
            ("Extract archive", "tar -xzvf archive.tar.gz"),
            ("List archive contents", "tar -tzvf archive.tar.gz"),
            ("Extract one file", "tar -xzvf archive.tar.gz folder/file.txt"),
        ],
    },
    ExplainEntry {
        binary: "ssh",
        summary: "ssh opens an encrypted shell on a remote host; keys in ~/.ssh/id_ed25519 (or id_rsa) are tried for auth before passwords.",
        examples: &[
            ("Log in", "ssh user@host"),
            ("Run one command", "ssh user@host 'uptime'"),
            ("Port forward local 8080 to remote 80", "ssh -L 8080:localhost:80 user@host"),
            ("Copy your key over", "ssh-copy-id user@host"),
        ],
    },
    ExplainEntry {
        binary: "rsync",
        summary: "rsync transfers only the differences between files — the standard tool for efficient syncing over SSH.",
        examples: &[
            ("Sync folder to a server", "rsync -avz --progress src/ user@host:/srv/src/"),
            ("Dry run (see what would happen)", "rsync -avzn src/ user@host:/srv/src/"),
            ("Mirror with deletions", "rsync -avz --delete src/ user@host:/srv/src/"),
        ],
    },
    ExplainEntry {
        binary: "du",
        summary: "du reports disk usage of files and directories.",
        examples: &[
            ("Size of each subdirectory", "du -h --max-depth=1 | sort -hr"),
            ("macOS variant", "du -h -d 1 | sort -hr"),
            ("Total of a folder", "du -sh folder/"),
        ],
    },
    ExplainEntry {
        binary: "find",
        summary: "find walks a directory tree and evaluates tests on every entry.",
        examples: &[
            ("Files larger than 100MB", "find . -type f -size +100M -exec ls -lh {} +"),
            ("Delete them (careful!)", "find . -type f -size +100M -delete"),
            ("Files changed in the last day", "find . -type f -mtime -1"),
        ],
    },
    ExplainEntry {
        binary: "grep",
        summary: "grep searches text using patterns (regular expressions by default).",
        examples: &[
            ("Search recursively", "grep -rn 'pattern' ."),
            ("Ignore case, list files", "grep -ril 'pattern' ."),
            ("Fixed string, whole words", "grep -rwnF 'pattern' ."),
        ],
    },
    ExplainEntry {
        binary: "ffmpeg",
        summary: "ffmpeg converts audio and video between formats; -i sets the input, the last argument is the output.",
        examples: &[
            ("Convert to mp4", "ffmpeg -i input.mov output.mp4"),
            ("Extract audio", "ffmpeg -i video.mp4 -vn -acodec copy audio.m4a"),
            ("Scale to 720p", "ffmpeg -i in.mp4 -vf scale=-2:720 out.mp4"),
        ],
    },
    ExplainEntry {
        binary: "curl",
        summary: "curl transfers data to/from URLs and speaks HTTP, FTP and many other protocols.",
        examples: &[
            ("GET with headers shown", "curl -v https://api.github.com"),
            ("POST JSON", "curl -X POST -H 'Content-Type: application/json' -d '{\"a\":1}' https://httpbin.org/post"),
            ("Download a file", "curl -fsSL -o install.sh https://example.com/install.sh"),
        ],
    },
    ExplainEntry {
        binary: "systemctl",
        summary: "systemctl controls systemd services (the init system on most Linux distros).",
        examples: &[
            ("Check a service", "systemctl status nginx"),
            ("Restart it", "sudo systemctl restart nginx"),
            ("Enable at boot", "sudo systemctl enable --now nginx"),
        ],
    },
];

/// Install hints for common "command not found" binaries.
pub static INSTALL_HINTS: &[(&str, &str)] = &[
    ("docker", "https://docs.docker.com/get-docker/ (or: curl -fsSL https://get.docker.com | sh)"),
    ("kubectl", "https://kubernetes.io/docs/tasks/tools/"),
    ("node", "Install via nvm: curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/master/install.sh | bash"),
    ("npm", "Comes with Node.js (install via nvm)"),
    ("jq", "sudo apt install jq  |  brew install jq"),
    ("htop", "sudo apt install htop  |  brew install htop"),
    ("fzf", "sudo apt install fzf  |  brew install fzf"),
    ("ffmpeg", "sudo apt install ffmpeg  |  brew install ffmpeg"),
    ("tree", "sudo apt install tree  |  brew install tree"),
    ("ripgrep", "sudo apt install ripgrep  |  brew install ripgrep"),
    ("rg", "sudo apt install ripgrep  |  brew install ripgrep"),
    ("tmux", "sudo apt install tmux  |  brew install tmux"),
    ("pip", "python3 -m ensurepip --user  (or install python3-pip via apt)"),
    ("pip3", "python3 -m ensurepip --user"),
    ("terraform", "https://developer.hashicorp.com/terraform/downloads"),
    ("pg_dump", "sudo apt install postgresql-client  |  brew install libpq"),
    ("psql", "sudo apt install postgresql-client  |  brew install libpq"),
    ("mysql", "sudo apt install mysql-client  |  brew install mysql-client"),
];
