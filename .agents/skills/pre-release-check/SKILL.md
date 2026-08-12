---
name: pre-release-check
description: Use before cutting a release. Runs the four check and run skills, then verifies the version bump and commit hygiene. Does not bump the version and does not tag.
---

# Pre-release check

A gate, not a fixer. Anything it finds gets fixed in its own commit before the
release proceeds; do not fold repairs into the release commit, which should
stay a mechanical, reviewable change.

## Run these five first

In this order, because a later one is pointless if an earlier one fails:

1. `run-tests-and-linters` -- the workspace is green, with captured output.
2. `check-licenses` -- every direct dependency is compliant and justified.
3. `regenerate-config-schema` -- `doppel-config.schema.json` matches the types
   and every field still describes itself. It is a release asset and is what
   editors fetch, so a stale one ships.
4. `check-changes` -- the Development section reflects what landed.
5. `check-docs` -- the reference, CLI surface, error codes and rule table match
   the code.

`bump-version` is deliberately not in this list. This skill checks; that one
changes things. `regenerate-config-schema` is the one exception, and only
because the thing it writes is generated: if it produces a diff, that diff is a
commit of its own before the release, not part of the release commit.

## Then check the dashboard

`run-tests-and-linters` covers the frontend gate, but a release has two questions
of its own, because the dashboard is embedded at compile time and a binary without
it still builds:

```bash
make e2e          # the browser suite, which the frontend gate leaves out
```

Then confirm the release will not ship a page-less binary. The workflow greps each
archive's binary for the configuration element before publishing -- read that step
and check it is still there, since it is the only thing standing between a missing
artifact and a release whose root answers 503:

```bash
grep -n "carries the dashboard" .github/workflows/release.yml
```

The dashboard's own version is not bumped and must not be: `frontend/package.json`
has no `version` field, and the page reports the binary's.

## Then check the Docker Hub overview

`DOCKERHUB.md` is the only page here that is published somewhere this repository
cannot see, by the `describe` job, and nothing fails when it goes stale: the
release ships and the overview quietly describes an older Doppel.

It is not a copy of `docs/usage/docker.md`. It is written for a reader who has
already chosen the image, so it opens with `docker run` and never mentions
building from source. What to check, in this order:

- **Against the code.** Every command, port, path, volume and environment
  variable in it exists and still means what it says. The paths are the ones that
  moved most recently, so read the route table rather than trusting the text.
- **Against `docs/usage/docker.md`.** The two cover the same ground at different
  lengths, and the failure mode is a fact that changed in one and not the other:
  a mount, a variable, a healthcheck path. Diff them by eye, fact by fact.
- **Against the dashboard.** It describes what the page can do, which is the
  claim that ages fastest -- it has already been wrong once about templates.
- **The version stays the placeholder `1.2.3`.** The `describe` job rewrites
  every `loremdev/doppel:<version>-alpine` to the tag being released; a real
  version checked in is a second number to bump and no reader benefits.
- **Size.** Docker Hub truncates the overview past 25000 bytes and the short
  description past 100, silently:

```bash
wc -c DOCKERHUB.md
grep -n 'loremdev/doppel:' DOCKERHUB.md
```

Anything found here is a commit of its own before the release, like everything
else this skill turns up.

## Then check the version

```bash
grep -n '^version' Cargo.toml
git tag --list | tail -5
```

The workspace version must be ahead of the newest release tag, and the
CHANGES.md heading for it must exist with a date. If the version is unchanged
since the last tag, the release has not been prepared -- stop and say so.

## Then dry-run the release scripts

The release workflow composes its body from these two. Running them here means
a failure lands before the tag rather than after, when the only fix is a second
tag.

```bash
uv run scripts/release_notes.py <version>
uv run scripts/release_downloads.py <version> <a directory of fake assets>
```

`release_notes.py` fails when CHANGES.md has no section for the version, or has
an empty one. `release_downloads.py` fails on an empty asset directory and puts
anything it cannot classify under "Other" rather than dropping it -- so read its
output rather than only its exit code, and check every platform you expect is
listed.

Read the composed `RELEASE_NOTES.md` before continuing, and delete it: it is a
build artifact, not a file the repository keeps.

## Then check that a release can actually be installed

The asset names are a contract. `scripts/install.sh` builds
`doppel-<target>.tar.gz` from a hardcoded pattern, and
`.github/workflows/release.yml` stages the archives under exactly that name.
Neither reads the other.

```bash
grep -n 'doppel-\${target}\|asset=' scripts/install.sh
grep -n 'asset=' .github/workflows/release.yml
```

The three targets in the `build` matrix, the three in `install.sh`'s `case`,
and the three in `release_downloads.py`'s `TARGETS` must be the same three. A
target added to the build and not to the installer produces an archive nobody
can install with one line, and nothing else notices.

The `musl` job's two targets are deliberately *not* in those three: they are
copied into the container image and are not release assets. What has to hold
for them is that the `image` job's `install` lines name the artifacts the
`musl` job produced -- a rename there fails the job, which is the good case,
but check it rather than assuming.

For the image itself:

```bash
grep -n 'images:\|tags:' -A6 .github/workflows/release.yml | sed -n '/metadata-action/,+12p'
```

`latest=false` must still be there. A moving tag is one an unpinned deployment
follows into a release nobody reviewed, and a pre-release would take it.

## Then check that every action reference resolves

`actionlint` validates syntax and inputs. It does not, and cannot without the
network, check that the tag you pinned exists. A reference to a tag that does
not is not caught until the job fails at "Set up job", before a single step
runs.

```bash
for ref in $(grep -rhoE 'uses: [a-zA-Z0-9/_.-]+@[a-zA-Z0-9._-]+' .github/workflows/ \
             | sed 's/uses: //' | sort -u); do
  repo="${ref%@*}"; tag="${ref#*@}"
  code=$(curl -o /dev/null -s -w '%{http_code}' \
         "https://api.github.com/repos/$repo/git/ref/tags/$tag")
  [ "$code" = "200" ] && echo "OK      $ref" || echo "MISSING $ref"
done
```

**Do not derive a major tag from a release number.** `releases/latest`
returning `v9.0.0` does not mean `@v9` exists: publishing a sliding major tag
is a convention, not a rule, and maintainers drop it. `astral-sh/setup-uv`
publishes `v9.0.0` and stops its major tags at `v7`; assuming otherwise put two
workflows in the repository that could not start. Ask for the ref you intend to
write.

## On the first release only: the two remaining badges

The README carries badges for docs, CI, Docs and the licence. Two more belong
there and are deliberately absent until there is something behind them, because
each renders as an error otherwise:

```html
<a href="https://github.com/lorem-dev/doppel/releases/latest"><img src="https://img.shields.io/github/v/release/lorem-dev/doppel?label=download" alt="Download"></a>
<a href="https://hub.docker.com/r/loremdev/doppel"><img src="https://img.shields.io/docker/v/loremdev/doppel?label=docker&sort=semver" alt="Docker"></a>
```

`no releases or repo not found` and `repository or tag not found` are what they
say today. Add them once the first release and the first image exist, and check
what they render before committing -- a download badge on a project with
nothing to download is worse than no badge.

## Then check the release key

The signature is worth nothing if the published public key has expired or no
longer matches what signs releases.

```bash
gpg --show-keys .github/release-key.asc
```

Read the expiry date, not just that the command succeeded. A key that expires
between this release and the next produces signatures nobody can verify, and
the failure appears on the *user's* machine rather than in this pipeline.

Signing is gated on `DOPPEL_RELEASE_GPG_KEY` being set, so a release without it
publishes unsigned and logs a warning rather than failing. Confirm the secret
exists before a release that is meant to be signed:

```bash
gh secret list --repo lorem-dev/doppel
```

## Then check commit hygiene

Over the range since the last tag:

```bash
git log --format='%H %s' <last-tag>..HEAD
git log --format='%B%n%(trailers)' <last-tag>..HEAD | grep -icE 'claude|codex|copilot|co-authored|generated with|assistant'
```

Every subject must be Conventional Commits -- one of `feat`, `fix`, `chore`,
`docs`, `test`, `refactor`, `perf`, `ci`, `build` -- in English, imperative,
under 72 characters, with a scope only if that scope already exists in the log.
The grep must return zero: no mention of AI tools, agents or assistants
anywhere in any message, body or trailer.

## Report

List each check with its result and the evidence you read. A pass
with no evidence behind it is the failure mode this project has seen most
often: several reports over its history quoted counts and diagnostics that did
not survive being checked. Name the command and the file for each figure.

State plainly whether the release should proceed. "Ready except X" is a useful
answer; "looks good" is not.
