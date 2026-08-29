# Example workflows

Real transcripts, produced by `./demo/demo.sh --no-pause` (offline mode —
no model required).

## 1. Git exploration without reading --help

```text
$ git log --                          ← typing…
$ git log --oneline --graph --decorate  ← ghost text from history

$ sm complete --shell zsh --buffer "git log --" --plain
--oneline       Show one commit per line          flag
--graph         Show the commit graph             flag
--author=       Filter by author                  flag
--since=        Show commits after date           flag
```

## 2. Docker: flags you didn't know existed

```text
$ docker images --<TAB>
--filter=   Filter output (e.g. dangling=true)   flag
--format=   Format output with a Go template     flag
-a          Include intermediate images          flag
```

The palette explains what it suggests:

```text
$ sm palette --query "show untagged images"
  docker images --filter dangling=true
  Shows untagged and unreferenced images.
```

## 3. Kubernetes with your real namespace

```text
$ kubectl get <TAB>
pods  deployments  services  configmaps  secrets  nodes  namespaces  ingresses …

$ kubectl get pods -n <TAB>          ← from the current kubeconfig context
production
```

## 4. Fixing a failed push (the flagship)

```text
$ git push origin main
fatal: The current branch main has no upstream branch.

$ sm fix
command: git push origin main

  1. git push --set-upstream origin main
     Your local main branch is not tracking a remote branch. This pushes
     main and sets it to track origin/main.
```

Even without captured stderr, shellmind *infers* this from the repo state
(a branch with no upstream configured).

## 5. Python venv amnesia

```text
$ python manage.py migrate
ModuleNotFoundError: No module named 'django'

$ sm fix --error "ModuleNotFoundError: No module named 'django'" python manage.py migrate
  1. source .venv/bin/activate
     Your Python virtual environment is not active — the module is
     probably installed inside it.
  2. pip install -r requirements.txt
  3. pip install django
```

## 6. "That command from last week"

```text
$ sm history "postgres backup command from last week"
history "postgres backup command from last week" (bm25)
   1. pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb
   2. tar -czvf archive.tar.gz folder/

$ sm history "docker remove unused images"
   1. docker image prune -a
```

Note the synonym expansion: *remove* matches `prune`, *images* matches
`image` — no exact substring anywhere.

## 7. Typos, caught

```text
$ gti status
zsh: command not found: gti

$ sm fix
  1. git
     'gti' is not installed, but 'git' is — did you mean this?
```

## 8. The safety net

```text
$ sm safety-check "rm -rf ./"
safety: destructive
! [DESTRUCTIVE] This recursively force-deletes everything matching in the
  current directory.
  safer alternatives:
    • rm -i -r ./
    • trash ./
    • git clean -n
  ⚠ confirmation required before execution
$ echo $?
2
```

## 9. Snippets with placeholders

```text
$ sm save "postgres backup" "pg_dump -U {{user}} -h {{host}} -F c -b -v -f {{file}} {{db}}"
saved snippet postgres backup

$ sm use "postgres backup" --set user=postgres --set host=localhost \
                            --set file=backup.dump --set db=mydb
pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb
```

## 10. Your aliases, remembered

```text
$ alias dps='docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"'

$ sm palette --query "show running containers"
  1. dps                        ← your alias, expansion preview attached
  2. docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
```
