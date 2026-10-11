# Access and syncing

## SSH without typing the IP

On your machine, once:

```bash
ssh-keygen -t ed25519 -N "" -f ~/.ssh/id_ed25519     # skip if you have a key
ssh-copy-id <user>@<thor-ip>                         # asks for the password once
cat >> ~/.ssh/config <<'EOF'

Host thor
    HostName <thor-ip>
    User <user>
    IdentityFile ~/.ssh/id_ed25519
    ControlMaster auto
    ControlPath ~/.ssh/cm-%r@%h:%p
    ControlPersist 10m
EOF
```

Then `ssh thor`, `scp file thor:`, or `ssh thor 'command'`.

## thor-sync

Keeps folders on your machine copied to the Thor. One way: your machine to
the Thor.

```bash
thor-sync host thor      # once: which device
cd ~/Projects/my-project
thor-sync add            # add this folder and sync it
thor-sync on             # keep syncing in the background, also after reboot
```

<figure>
<img src="figures/thor-sync-flow.svg" alt="On your machine, thor-sync watches the folders on its list, reads its host, folder, exclude and include files from ~/.config/thor-sync, and copies each folder with rsync over ssh, one way, to the same place under the Thor's home folder. It skips what .gitignore and the exclude list skip, and never deletes on the Thor.">
<figcaption>What <code>thor-sync</code> copies, and where it lands.</figcaption>
</figure>

`thor-sync` is one bash script around `rsync`, run over the `ssh thor` alias
above. Three choices in it are worth knowing before you rely on it.

- **It copies one way.** Your machine is the source and the Thor is the copy.
  That is why a commit made on the Thor is lost: the next sync replaces the
  Thor's `.git` with yours.
- **It never deletes.** `rsync` is run without `--delete`, so a file you remove
  locally stays on the Thor until you remove it there. A renamed file exists
  under both names on the Thor.
- **It follows `.gitignore`.** Build output and model files are not copied.
  Something ignored that the Thor does need, such as `data/`, goes in the
  include list.

| Command | Does |
|---|---|
| `thor-sync` | sync everything on the list |
| `thor-sync ls` / `rm` | show the list / remove the current folder |
| `thor-sync off` / `log` | stop background sync / follow it |

- Folders land at the same place under the Thor's home folder.
- Git history is copied, so `git` works there. Commit on your machine; a
  commit made on the Thor is overwritten by the next sync.
- Skipped: whatever `.gitignore` skips, plus `~/.config/thor-sync/exclude`
  (build output, model files). `~/.config/thor-sync/include` copies ignored
  paths anyway, e.g. `/data/***`.
- Deleting a file locally never deletes it on the Thor.
