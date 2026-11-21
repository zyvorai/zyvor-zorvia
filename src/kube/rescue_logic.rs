// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Pure, dependency-free helpers shared by the server (`src/kube/rescue.rs`,
//! request validation + Job-spec generation) and the `rescue-agent` binary
//! (which `#[path]`-includes this file so it can be unit-tested without the
//! optional `guestkit` dependency). Nothing here touches a disk, the network
//! or a cluster. See docs/RESCUE.md.

/// Most packages one install request may name.
pub const MAX_PACKAGES: usize = 20;
/// Longest single package name accepted.
pub const MAX_PACKAGE_LEN: usize = 100;
/// Longest password accepted (bytes).
pub const MAX_PASSWORD_LEN: usize = 128;
/// Hard cap on the result line the server will accept from a Job pod's logs.
pub const MAX_RESULT_BYTES: usize = 16 * 1024;
/// Longest single string in an inspect summary / result message.
pub const MAX_TEXT_LEN: usize = 256;
/// Most entries in any list inside an inspect summary.
pub const MAX_LIST_ITEMS: usize = 50;

/// A plain Linux account name: 1-32 chars of `[A-Za-z0-9_-]`.
pub fn validate_username(user: &str) -> Result<&str, &'static str> {
    let user = user.trim();
    if user.is_empty() || user.len() > 32 {
        return Err("user must be 1-32 characters");
    }
    if !user
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("user must be a plain username");
    }
    Ok(user)
}

/// Passwords are written verbatim (no trimming): 1-128 bytes, no control
/// characters (a newline would break the `/etc/shadow` line and the
/// `openssl passwd -stdin` contract).
pub fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.is_empty() || password.len() > MAX_PASSWORD_LEN {
        return Err("password must be 1-128 characters");
    }
    if password.chars().any(|c| c.is_control()) {
        return Err("password must not contain control characters");
    }
    Ok(())
}

/// Strict package-name pattern: starts with an alphanumeric (so it can never
/// be parsed as an option), then `[A-Za-z0-9._+-]`, at most 100 chars. This
/// covers deb/rpm/apk names; it deliberately excludes whitespace, `=`, `:`,
/// `/`, `*` and every shell metacharacter.
pub fn validate_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    name.len() <= MAX_PACKAGE_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// Validates and de-duplicates a package list (order preserved).
pub fn validate_packages(packages: &[String]) -> Result<Vec<String>, &'static str> {
    if packages.is_empty() {
        return Err("at least one package is required");
    }
    if packages.len() > MAX_PACKAGES {
        return Err("at most 20 packages per request");
    }
    let mut out: Vec<String> = Vec::new();
    for p in packages {
        let p = p.trim();
        if !validate_package_name(p) {
            return Err("package names must match [A-Za-z0-9][A-Za-z0-9._+-]{0,99}");
        }
        if !out.iter().any(|e| e == p) {
            out.push(p.to_string());
        }
    }
    Ok(out)
}

/// Replaces the password field of `user`'s `/etc/shadow` line with `hash`.
/// Every other field and line is preserved byte-for-byte. Errors if the user
/// has no entry (this never creates accounts) or the hash is not a plain
/// `$id$...` crypt string.
pub fn set_shadow_hash(shadow: &str, user: &str, hash: &str) -> Result<String, &'static str> {
    if !hash.starts_with('$') || hash.contains([':', '\n', '\r', '\0']) || hash.len() < 8 {
        return Err("password hash is malformed");
    }
    let mut found = false;
    let mut out = String::with_capacity(shadow.len() + hash.len());
    for piece in shadow.split_inclusive('\n') {
        let (line, nl) = match piece.strip_suffix('\n') {
            Some(l) => (l, "\n"),
            None => (piece, ""),
        };
        let mut fields: Vec<&str> = line.split(':').collect();
        if !found && fields.len() >= 2 && fields[0] == user {
            fields[1] = hash;
            found = true;
            out.push_str(&fields.join(":"));
        } else {
            out.push_str(line);
        }
        out.push_str(nl);
    }
    if found {
        Ok(out)
    } else {
        Err("user has no entry in /etc/shadow")
    }
}

/// Guest package managers the install operation knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    Yum,
    Zypper,
    Apk,
}

impl PackageManager {
    /// Guest-absolute binary paths probed (in order) to detect the manager.
    pub fn probes() -> &'static [(&'static str, PackageManager)] {
        &[
            ("/usr/bin/apt-get", PackageManager::Apt),
            ("/usr/bin/dnf", PackageManager::Dnf),
            ("/usr/bin/yum", PackageManager::Yum),
            ("/usr/bin/zypper", PackageManager::Zypper),
            ("/sbin/apk", PackageManager::Apk),
        ]
    }

    /// Explicit argv vectors (no shell) run via chroot, in order. Package
    /// names must already have passed [`validate_packages`].
    pub fn install_commands(self, packages: &[String]) -> Vec<Vec<String>> {
        let v = |parts: &[&str]| -> Vec<String> { parts.iter().map(|s| s.to_string()).collect() };
        let mut install = match self {
            Self::Apt => v(&[
                "/usr/bin/env",
                "DEBIAN_FRONTEND=noninteractive",
                "/usr/bin/apt-get",
                "install",
                "-y",
                "--no-install-recommends",
            ]),
            Self::Dnf => v(&["/usr/bin/dnf", "install", "-y"]),
            Self::Yum => v(&["/usr/bin/yum", "install", "-y"]),
            Self::Zypper => v(&["/usr/bin/zypper", "--non-interactive", "install"]),
            Self::Apk => v(&["/sbin/apk", "add", "--no-cache"]),
        };
        install.extend(packages.iter().cloned());
        let mut cmds = Vec::new();
        if self == Self::Apt {
            cmds.push(v(&[
                "/usr/bin/env",
                "DEBIAN_FRONTEND=noninteractive",
                "/usr/bin/apt-get",
                "update",
            ]));
        }
        cmds.push(install);
        cmds
    }
}

/// Pseudo-filesystems bind-mounted into the guest chroot, in mount order
/// (`(source on the Job pod, directory under the guest root)`). Maintainer
/// scripts commonly need `/proc` (e.g. `mount`, `ps`), `/dev` (`/dev/null`,
/// `/dev/urandom`) and `/sys`. Unmounting is always the reverse order.
pub const CHROOT_BINDS: [(&str, &str); 3] = [("/proc", "proc"), ("/dev", "dev"), ("/sys", "sys")];

/// Abstraction over "run one privileged command" so the mount/unmount
/// ordering can be unit-tested without root. The agent's real implementation
/// shells out; tests record the argv instead.
pub trait CommandRunner {
    fn run(&mut self, argv: &[String]) -> Result<(), String>;
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// `mount --bind <src> <root>/<dir>` argv for one bind.
pub fn bind_mount_argv(root: &str, source: &str, dir: &str) -> Vec<String> {
    let target = format!("{}/{}", root.trim_end_matches('/'), dir);
    argv(&["mount", "--bind", source, &target])
}

/// `umount <root>/<dir>` argv for one bind.
pub fn unmount_argv(root: &str, dir: &str, lazy: bool) -> Vec<String> {
    let target = format!("{}/{}", root.trim_end_matches('/'), dir);
    if lazy {
        argv(&["umount", "-l", &target])
    } else {
        argv(&["umount", &target])
    }
}

/// Bind-mounts /proc, /dev and /sys into `root`, runs `body`, then unmounts
/// them in reverse order. Cleanup happens on every path: if the Nth mount
/// fails, the N-1 already done are unmounted before returning; if `body`
/// fails, the unmounts still run. An unmount that fails is retried once
/// lazily (`umount -l`, for a busy mount left by a daemon the maintainer
/// scripts started). A leftover mount is reported as an error (appended to
/// any body error) because it would make GuestKit's own unmount fail.
pub fn with_chroot_binds<R: CommandRunner, T>(
    runner: &mut R,
    root: &str,
    body: impl FnOnce(&mut R) -> Result<T, String>,
) -> Result<T, String> {
    let mut mounted: Vec<&str> = Vec::new();
    let mut setup_err = None;
    for (source, dir) in CHROOT_BINDS {
        if let Err(e) = runner.run(&bind_mount_argv(root, source, dir)) {
            setup_err = Some(format!("could not bind-mount {source} into the guest: {e}"));
            break;
        }
        mounted.push(dir);
    }
    let result = match setup_err {
        Some(e) => Err(e),
        None => body(runner),
    };
    let mut leftovers = Vec::new();
    for dir in mounted.iter().rev() {
        if runner.run(&unmount_argv(root, dir, false)).is_err()
            && runner.run(&unmount_argv(root, dir, true)).is_err()
        {
            leftovers.push(*dir);
        }
    }
    if leftovers.is_empty() {
        return result;
    }
    let msg = format!(
        "could not unmount /{} from the guest",
        leftovers.join(", /")
    );
    match result {
        Ok(_) => Err(msg),
        Err(e) => Err(format!("{e}; {msg}")),
    }
}

/// Makes a string safe to return to a caller: drops control characters,
/// masks `key=value` tokens whose key looks sensitive and any long
/// token-looking run, then truncates to `max` chars.
pub fn redact_text(s: &str, max: usize) -> String {
    const SENSITIVE: [&str; 7] = ["pass", "secret", "token", "key", "cred", "auth", "private"];
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words: Vec<String> = cleaned
        .split_whitespace()
        .map(|w| {
            if let Some((k, _)) = w.split_once('=') {
                let lk = k.to_ascii_lowercase();
                if SENSITIVE.iter().any(|s| lk.contains(s)) {
                    return format!("{k}=[redacted]");
                }
            }
            let tokenish = w.len() >= 40
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'));
            if tokenish {
                "[redacted]".to_string()
            } else {
                w.to_string()
            }
        })
        .collect();
    let joined = words.join(" ");
    if joined.chars().count() > max {
        let mut t: String = joined.chars().take(max).collect();
        t.push('…');
        t
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn package_pattern_is_strict() {
        for ok in [
            "curl",
            "libstdc++6",
            "python3.11",
            "g++",
            "nginx-extras",
            "a",
        ] {
            assert!(validate_package_name(ok), "{ok}");
        }
        for bad in [
            "", "-y", "--force", "a b", "a;b", "$(id)", "a`b`", "a|b", "a&b", "a=1.0", "a:amd64",
            "../x", "a/b", "a*", ".hidden", "+x", "naïve", "a\nb",
        ] {
            assert!(!validate_package_name(bad), "{bad:?}");
        }
        assert!(!validate_package_name(&"a".repeat(101)));
        assert!(validate_package_name(&"a".repeat(100)));
    }

    #[test]
    fn package_list_bounds_and_dedup() {
        assert!(validate_packages(&[]).is_err());
        assert_eq!(
            validate_packages(&s(&["curl", " curl ", "git"])).unwrap(),
            s(&["curl", "git"])
        );
        let many: Vec<String> = (0..21).map(|i| format!("p{i}")).collect();
        assert!(validate_packages(&many).is_err());
        assert!(validate_packages(&s(&["ok", "--bad"])).is_err());
    }

    #[test]
    fn username_and_password_rules() {
        assert_eq!(validate_username(" root ").unwrap(), "root");
        assert!(validate_username("a:b").is_err());
        assert!(validate_username("").is_err());
        assert!(validate_username(&"a".repeat(33)).is_err());
        assert!(validate_password("hunter2 with spaces!").is_ok());
        assert!(validate_password("").is_err());
        assert!(validate_password("a\nb").is_err());
        assert!(validate_password(&"x".repeat(129)).is_err());
    }

    #[test]
    fn shadow_rewrite_touches_only_target_user() {
        let shadow =
            "root:!:19000:0:99999:7:::\nbob:$6$old$x:19000:0:99999:7:::\ndaemon:*:1:::::\n";
        let out = set_shadow_hash(shadow, "bob", "$6$salt$newhashvalue").unwrap();
        assert_eq!(
            out,
            "root:!:19000:0:99999:7:::\nbob:$6$salt$newhashvalue:19000:0:99999:7:::\ndaemon:*:1:::::\n"
        );
        // no trailing newline preserved
        let out = set_shadow_hash("root:x:1", "root", "$6$salt$newhashvalue").unwrap();
        assert_eq!(out, "root:$6$salt$newhashvalue:1");
        assert!(set_shadow_hash(shadow, "ghost", "$6$salt$newhashvalue").is_err());
        // "bo" must not match "bob"
        assert!(set_shadow_hash(shadow, "bo", "$6$salt$newhashvalue").is_err());
        assert!(set_shadow_hash(shadow, "bob", "plain").is_err());
        assert!(set_shadow_hash(shadow, "bob", "$6$a:b$cccccc").is_err());
    }

    #[test]
    fn install_commands_are_plain_argv() {
        let pk = s(&["curl", "git"]);
        let apt = PackageManager::Apt.install_commands(&pk);
        assert_eq!(apt.len(), 2);
        assert_eq!(apt[0].last().unwrap(), "update");
        assert_eq!(&apt[1][apt[1].len() - 2..], &s(&["curl", "git"])[..]);
        assert!(apt[1].contains(&"--no-install-recommends".to_string()));
        let apk = PackageManager::Apk.install_commands(&pk);
        assert_eq!(apk.len(), 1);
        assert_eq!(apk[0][..3], s(&["/sbin/apk", "add", "--no-cache"])[..]);
        assert_eq!(PackageManager::Dnf.install_commands(&pk)[0][1], "install");
    }

    #[test]
    fn redaction_masks_and_bounds() {
        assert_eq!(redact_text("a\u{7}b\nc", 50), "a b c");
        assert_eq!(
            redact_text("DB_PASSWORD=hunter2 ok", 50),
            "DB_PASSWORD=[redacted] ok"
        );
        assert_eq!(redact_text("api_key=abc", 50), "api_key=[redacted]");
        let tok = "A".repeat(48);
        assert_eq!(redact_text(&format!("x {tok}"), 50), "x [redacted]");
        let long = redact_text(&"word ".repeat(200), 20);
        assert!(long.chars().count() <= 21);
    }
    #[derive(Default)]
    struct Recorder {
        calls: Vec<String>,
        fail_on: Option<String>,
    }
    impl CommandRunner for Recorder {
        fn run(&mut self, a: &[String]) -> Result<(), String> {
            let line = a.join(" ");
            self.calls.push(line.clone());
            match &self.fail_on {
                Some(f) if line.contains(f.as_str()) => Err("boom".into()),
                _ => Ok(()),
            }
        }
    }

    #[test]
    fn binds_mount_in_order_and_unmount_in_reverse() {
        let mut r = Recorder::default();
        let out = with_chroot_binds(&mut r, "/mnt/g/", |r| {
            r.run(&s(&["chroot", "/mnt/g", "apt-get", "install"]))?;
            Ok(7)
        });
        assert_eq!(out, Ok(7));
        assert_eq!(
            r.calls,
            [
                "mount --bind /proc /mnt/g/proc",
                "mount --bind /dev /mnt/g/dev",
                "mount --bind /sys /mnt/g/sys",
                "chroot /mnt/g apt-get install",
                "umount /mnt/g/sys",
                "umount /mnt/g/dev",
                "umount /mnt/g/proc",
            ]
        );
    }

    #[test]
    fn body_failure_still_unmounts() {
        let mut r = Recorder::default();
        let out: Result<(), String> =
            with_chroot_binds(&mut r, "/r", |_| Err("install failed".into()));
        assert_eq!(out, Err("install failed".into()));
        assert_eq!(
            &r.calls[3..],
            ["umount /r/sys", "umount /r/dev", "umount /r/proc"]
        );
    }

    #[test]
    fn mount_failure_unwinds_only_what_was_mounted_and_skips_body() {
        let mut r = Recorder {
            fail_on: Some("--bind /sys".into()),
            ..Default::default()
        };
        let mut ran = false;
        let out: Result<(), String> = with_chroot_binds(&mut r, "/r", |_| {
            ran = true;
            Ok(())
        });
        assert!(!ran);
        assert!(out.unwrap_err().contains("bind-mount /sys"));
        assert_eq!(
            r.calls,
            [
                "mount --bind /proc /r/proc",
                "mount --bind /dev /r/dev",
                "mount --bind /sys /r/sys",
                "umount /r/dev",
                "umount /r/proc",
            ]
        );
    }

    #[test]
    fn busy_unmount_retries_lazily_and_leftover_is_an_error() {
        let mut r = Recorder {
            fail_on: Some("umount /r/dev".into()),
            ..Default::default()
        };
        let out: Result<(), String> = with_chroot_binds(&mut r, "/r", |_| Ok(()));
        // "umount -l /r/dev" does not contain "umount /r/dev", so lazy succeeds.
        assert_eq!(out, Ok(()));
        assert!(r.calls.contains(&"umount -l /r/dev".to_string()));

        let mut r = Recorder {
            fail_on: Some("/r/dev".into()),
            ..Default::default()
        };
        let out: Result<(), String> = with_chroot_binds(&mut r, "/r", |_| Err("x".into()));
        let e = out.unwrap_err();
        assert!(e.starts_with("x; ") && e.contains("/dev"), "{e}");
    }
}
