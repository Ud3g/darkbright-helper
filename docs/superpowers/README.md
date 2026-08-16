# AI-assisted development artifacts

The design documents in [`specs/`](specs/) come out of this project's AI-assisted
development process (see [How this project was built](../../README.md#how-this-project-was-built)
in the top-level README). Each one records, for a single implementation cycle: the problem,
the design decisions with their rationale, and any amendments made after adversarial review.

They are **dated snapshots**, deliberately left as written — they document why the code took
the shape it did at that point in time. For the current state of the system,
[`docs/architecture.md`](../architecture.md) is authoritative.

The step-by-step implementation *plans* that accompanied these specs were one-shot
execution scripts for AI agents. Having served their purpose, they are not kept in the
tree (the `plans/` directory is gitignored so future working plans stay local); the
executed ones remain available in the git history.
