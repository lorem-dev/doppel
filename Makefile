# Doppel, in one command at a time.
#
# Nothing here is required to work on the project -- every target is a short
# sequence of `cargo`, `npm` and `uv` that could be typed out instead. What it
# buys is the ordering that is easy to get wrong: the dashboard is embedded into
# the binary at compile time, so the assets have to be built *before* cargo runs,
# and the image needs a musl binary staged where the Dockerfile expects it.
#
# `make` on its own prints the list.

CARGO ?= cargo
NPM ?= npm
UV ?= uv
FRONTEND ?= frontend

# The image's tag. Override for a real publish: `make image IMAGE=loremdev/doppel:1.2.3-alpine`.
IMAGE ?= doppel:dev

# Nothing here sets a target for an ordinary build: `cargo build` produces a binary
# for the machine it runs on, which is what `build` and `release` mean on any
# platform.
#
# The image needs a Linux one, and that is the Dockerfile's problem rather than
# this file's: it uses a binary staged in `dist/` when there is one and compiles
# inside its own builder stage when there is not. Which is what makes `make image`
# work on a Mac, where `ring` cannot be linked against musl without a cross
# toolchain nobody has installed.

# A database for the store suites. The port is deliberately not 5432; see
# docker-compose.yml.
TEST_DATABASE_URL ?= postgres://doppel:doppel@127.0.0.1:55432/doppel

.DEFAULT_GOAL := help

# Every target here is a name, not a file, so make must not go looking for one
# and must not skip a target because a directory of that name happens to exist --
# `frontend`, `docs` and `dist` all do.
.PHONY: build release frontend run gate fmt fmt-check lint test test-db \
        test-frontend size e2e schema schema-write licences licences-write \
        docs docs-serve image image-rebuild image-run db-up db-down migrate \
        clean clean-frontend help

# ---------------------------------------------------------------------------
# Building
# ---------------------------------------------------------------------------

build: frontend ## Build the debug binary with the dashboard embedded
	$(CARGO) build

release: frontend ## Build the release binary with the dashboard embedded
	$(CARGO) build --release

frontend: ## Build the dashboard into frontend/dist
	$(NPM) --prefix $(FRONTEND) ci
	$(NPM) --prefix $(FRONTEND) run build

run: build ## Run the debug binary against ./config/main.yaml
	./target/debug/doppel serve --config config/main.yaml

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

gate: fmt-check lint test test-frontend size docs schema parameters links licences ## Everything CI checks, except the browser suite
	@echo "gate: clean"

fmt: ## Format the Rust sources
	$(CARGO) fmt --all

fmt-check: ## Fail if anything is unformatted
	$(CARGO) fmt --all --check

lint: ## Clippy with warnings as errors, eslint, and both TypeScript projects
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(NPM) --prefix $(FRONTEND) run lint
	$(NPM) --prefix $(FRONTEND) run typecheck
	cd $(FRONTEND) && npx tsc --noEmit -p tsconfig.e2e.json

# The dashboard is built first and required, so the asset-delivery suites cannot
# skip themselves the way they may on a machine with no Node. Same reasoning as
# DOPPEL_REQUIRE_DATABASE, which is set only when a database is actually up.
test: frontend ## Run the Rust suites, requiring the embedded dashboard
	DOPPEL_REQUIRE_DASHBOARD_ASSETS=1 $(CARGO) test --workspace

test-db: frontend ## Run the Rust suites with the PostgreSQL store required
	DOPPEL_REQUIRE_DASHBOARD_ASSETS=1 \
	DOPPEL_REQUIRE_DATABASE=1 \
	DOPPEL_TEST_DATABASE_URL=$(TEST_DATABASE_URL) \
	$(CARGO) test --workspace

test-frontend: frontend ## Run the jest suites and the bundle-size budgets
	$(NPM) --prefix $(FRONTEND) test

size: frontend ## Report what the dashboard weighs, gzipped
	@cd $(FRONTEND) && node -e "\
		const { gzipSync } = require('node:zlib'); \
		const { readdirSync, readFileSync } = require('node:fs'); \
		const dir = 'dist/assets'; let total = 0; \
		for (const file of readdirSync(dir).sort()) { \
			const kb = gzipSync(readFileSync(dir + '/' + file)).length / 1024; \
			total += kb; \
			console.log(kb.toFixed(1).padStart(7) + ' KB  ' + file); \
		} \
		console.log(total.toFixed(1).padStart(7) + ' KB  total'); \
	"

# Not part of `gate`: it downloads a browser on first run and takes half a minute,
# which is the wrong trade for a check meant to be run constantly.
e2e: build ## Drive the built binary with Playwright
	cd $(FRONTEND) && npx playwright install chromium
	$(NPM) --prefix $(FRONTEND) run e2e

schema: ## Fail if doppel-config.schema.json is stale
	$(UV) run scripts/config_schema.py --check

schema-write: ## Regenerate doppel-config.schema.json
	$(UV) run scripts/config_schema.py

licences: ## Fail if THIRD-PARTY.md is stale, or a licence is outside the policy
	$(UV) run scripts/third_party.py --check

licences-write: frontend ## Regenerate THIRD-PARTY.md from both dependency graphs
	$(UV) run scripts/third_party.py

# ---------------------------------------------------------------------------
# Documentation
# ---------------------------------------------------------------------------

parameters: ## Fail if the generated parameter reference is stale
	$(UV) run scripts/parameters_doc.py --check

links: ## Fail if the dashboard links to documentation that is not there
	$(UV) run scripts/check_docs_links.py

docs: ## Build the documentation site, strictly
	$(UV) run --with-requirements docs/requirements.txt mkdocs build --strict

docs-serve: ## Serve the documentation with live reload
	$(UV) run --with-requirements docs/requirements.txt mkdocs serve

# ---------------------------------------------------------------------------
# The image
# ---------------------------------------------------------------------------

# The Dockerfile takes an already-built binary rather than compiling inside a
# multi-architecture build, where the non-native half runs under emulation and
# turns a two-minute compile into most of an hour. So the binary is built first,
# for this machine's architecture, and staged where the default BIN points.
image: frontend ## Build the container image for this architecture
	docker build -t $(IMAGE) .

# `rm -rf dist` as well as `--no-cache`: a binary left there from an earlier build
# is what the image would use, and "from scratch" has to mean the compile happens.
image-rebuild: clean-frontend ## Build the image from scratch: no npm or docker cache
	$(NPM) --prefix $(FRONTEND) ci
	$(NPM) --prefix $(FRONTEND) run build
	rm -rf dist
	docker build --no-cache -t $(IMAGE) .

image-run: ## Run the built image against ./config/main.yaml
	docker run --rm -p 8080:8080 -p 8081:8081 \
		-v "$(PWD)/config:/etc/doppel:ro" \
		$(IMAGE)

# ---------------------------------------------------------------------------
# The development database
# ---------------------------------------------------------------------------

db-up: ## Start PostgreSQL and wait for it
	docker compose up -d --wait postgres

db-down: ## Stop PostgreSQL and remove its volume
	docker compose down -v

migrate: build ## Apply the migrations to the development database
	DOPPEL_DATABASE_URL=$(TEST_DATABASE_URL) ./target/debug/doppel config migrate

# ---------------------------------------------------------------------------
# Cleaning
# ---------------------------------------------------------------------------

clean: clean-frontend ## Remove every build artifact
	$(CARGO) clean
	rm -rf dist site

clean-frontend: ## Remove the dashboard's build output
	rm -rf $(FRONTEND)/dist $(FRONTEND)/playwright-report $(FRONTEND)/test-results

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------

# Every target documented with `## ...` on its own line, in file order, so the
# list cannot drift from the targets: a target without a comment simply does not
# appear, and one that is renamed takes its description with it.
help: ## Print this list
	@printf 'Doppel. Targets:\n\n'
	@grep -hE '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN { FS = ":.*?## " } { printf "  \033[1m%-16s\033[0m %s\n", $$1, $$2 }'
	@printf '\nThis machine: %s. An ordinary build targets it. The image needs a\n' \
		'$(shell uname -s -m)'
	@printf 'Linux binary, which the Dockerfile compiles inside unless dist/ holds one.\n'
	@printf '\nVariables: IMAGE=%s\n' '$(IMAGE)'
