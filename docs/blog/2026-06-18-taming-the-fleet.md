    # Taming the Fleet: Two Small Fixes That Made an AI-Agent Orchestrator Trustworthy

    ### How I stopped a dashboard of parallel coding agents from crying wolf — and from fighting each other over port 3000

    ---

    > **TL;DR** — I run a small Rust daemon called **Kaiju** that launches many AI coding agents in parallel, each in its own git worktree and tmux session. Two reliability bugs were quietly eroding my trust in it: the dashboard kept raising false *"agent needs your input"* alerts, and any two agents that ran `npm run dev` collided on port 3000. The fixes were tiny — one new `i64` parameter and one ~40-line module — but they're a nice case study in *where* correctness lives: **in pure functions you can test, not in the IO you can't.**

    ---

    ## The setup: a kaiju is loose in your terminal

    If you've ever run more than one coding agent at once, you know the feeling. You kick off Claude in one terminal, Codex in another, maybe a Gemini session for a third task — and within ten minutes you've lost the plot. Which one is working? Which one is stuck? Which one is sitting at a prompt waiting for you to type `y`?

    Kaiju is my answer to that. It's a daemon that treats agents like a *fleet*:

    ```mermaid
    flowchart LR
        subgraph Browser
            D[Dashboard]
        end
        subgraph Daemon["kaiju-daemon (Axum)"]
            API[HTTP API]
            MON[Background Monitor]
            STORE[(Agent Store)]
        end
        subgraph OS
            T1[tmux: kaiju-claude-a1b2]
            T2[tmux: kaiju-codex-c3d4]
            T3[tmux: kaiju-gemini-e5f6]
        end

        D <-->|poll /agents| API
        API --> STORE
        MON -->|capture-pane| T1 & T2 & T3
        MON -->|status + metrics| STORE
        T1 -. git worktree .-> W1[/branch kaiju-a1b2/]
        T2 -. git worktree .-> W2[/branch kaiju-c3d4/]
        T3 -. git worktree .-> W3[/branch kaiju-e5f6/]
    ```

    Each agent gets:

    - its own **tmux session** (`kaiju-<type>-<id>`), so its process is the session's main process — when the agent exits, the session dies, and that's a clean "completed" signal;
    - its own **git worktree** (branch `kaiju/<id>`), so agents editing the same repo never step on each other;
    - a row on a **dashboard** that shows live status, runtime, tokens, and cost.

    The architecture follows a strict dependency rule — domain types (`kaiju-core`) know nothing about tmux, Axum, or git. The daemon's **monitor** is the one component that reaches out and touches the messy real world: every couple of seconds it captures each agent's terminal output and decides what state the agent is in.

    And that monitor is exactly where my trust was leaking away.

    ---

    ## Problem 1: the dashboard that cried "needs input"

    ### How status detection actually works

    The naive way to detect status is to scrape the terminal and pattern-match: *"if the last line looks like a prompt, the agent is waiting."* That's fragile — terminals redraw, spinners animate, prompts flicker past mid-stream.

    Kaiju already did something smarter, borrowed from the [Claude Squad](https://github.com/smtg-ai/claude-squad) approach: **content-hash activity detection.** Every poll, it fingerprints the pane and tracks how long the output has been *unchanged*:

    ```rust
    /// A cheap content fingerprint of pane output, to detect whether it changed.
    fn fingerprint(output: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        output.hash(&mut hasher);
        hasher.finish()
    }
    ```

    If the fingerprint hasn't changed in 120 seconds, the agent is probably **stuck**. If it changes again, it's back to **running**. That part was solid.

    The fragile part was the *other* signal. The adapter's output parser can spot a prompt-looking line and shout `WaitingForInput`. The old code trusted that **instantly** — and fired a desktop notification. But agents stream output in bursts. A line that *looks* like a prompt often scrolls past while the agent is still mid-thought:

    ```mermaid
    sequenceDiagram
        participant A as Agent (streaming)
        participant M as Monitor
        participant U as You

        A->>M: "...Proceed? (y/n)"  (still printing!)
        Note over M: OLD: prompt detected → trust it NOW
        M-->>U: 🔔 "Agent needs your input!"
        A->>M: "...just kidding, continuing"
        Note over U: 😤 looks over, nothing to do
    ```

    Cry wolf enough times and you stop looking. A status detector you don't believe is worse than no status detector — it's noise that *costs* attention.

    ### The fix: make the prompt prove it

    The insight: I already compute idle time for the stuck-detection logic. **I can reuse it as a confirmation gate.** A *fresh* waiting signal shouldn't be trusted until the pane has actually gone quiet for a few seconds — if output is still moving, that "prompt" is just a frame in a stream.

    So `resolve_status` — the pure heart of the monitor — grew one parameter, `wait_confirm`:

    ```rust
    pub fn resolve_status(
        current: AgentStatus,
        parsed: Option<AgentStatus>,
        idle_secs: i64,
        threshold: i64,
        wait_confirm: i64,
    ) -> AgentStatus {
        let base = parsed.unwrap_or(current);
        match base {
            // Only trust a freshly-detected prompt once the pane has actually gone
            // quiet; until then the agent is still considered active.
            AgentStatus::WaitingForInput
                if current != AgentStatus::WaitingForInput && idle_secs < wait_confirm =>
            {
                AgentStatus::Running
            }
            AgentStatus::Running if idle_secs >= threshold => AgentStatus::Stuck,
            AgentStatus::Stuck if idle_secs < threshold => AgentStatus::Running,
            other => other,
        }
    }
    ```

    Read the match arms top to bottom — it's a tiny state machine:

    ```mermaid
    stateDiagram-v2
        [*] --> Running
        Running --> Stuck: idle ≥ 120s
        Stuck --> Running: output moved
        Running --> WaitingForInput: prompt seen<br/>AND quiet ≥ 3s
        Running --> Running: prompt seen<br/>but still streaming
        WaitingForInput --> WaitingForInput: stays put<br/>(never re-flips)
    ```

    Three rules, carefully ordered:

    1. A **new** `WaitingForInput` signal is held back to `Running` while output is still moving (`idle < wait_confirm`). The prompt has to survive a few quiet seconds to be believed.
    2. An agent **already** waiting stays waiting — a prompt redrawing itself shouldn't bounce it back to "running."
    3. Everything else (errors, completion) passes straight through. A waiting agent is *never* re-labelled stuck.

    `WAIT_CONFIRM_SECS = 3`. That's the whole feature. The notification only fires on a *confirmed* transition, so the wolf-crying stops.

    ### Why this is the part that matters

    `resolve_status` is a **pure function**: same inputs, same output, no IO. That's deliberate. The monitor's `poll_once` does the untestable stuff — shelling out to tmux, hashing panes, writing to the store — but every *decision* is funneled through this one pure function. Which means the gnarly edge cases get nailed down as plain unit tests:

    ```rust
    #[test]
    fn fresh_waiting_signal_is_held_until_output_is_quiet() {
        let next = resolve_status(
            AgentStatus::Running, Some(AgentStatus::WaitingForInput),
            W - 1, T, W,
        );
        assert_eq!(next, AgentStatus::Running);
    }

    #[test]
    fn waiting_signal_is_trusted_once_output_is_quiet() {
        let next = resolve_status(
            AgentStatus::Running, Some(AgentStatus::WaitingForInput),
            W, T, W,
        );
        assert_eq!(next, AgentStatus::WaitingForInput);
    }
    ```

    No tmux, no sleeping, no flaky timing. Just a truth table. **Push the logic into something you can interrogate, and the IO becomes a thin shell you barely have to test.**

    ---

    ## Problem 2: two agents, one port 3000

    The second bug was dumber and more annoying. Isolated agents run in their own worktree, and front-end agents love to run a dev server:

    ```bash
    npm run dev    # Next.js → :3000
    next dev        # → :3000
    vite            # → :5173
    ```

    Run two of those at once and the second one either crashes or silently grabs `:3001` and now you have no idea which browser tab is which. Three agents, three guesses.

    ### The fix: hand each agent a port it can't fight over

    When the daemon launches an **isolated** agent, it now reserves a free port and injects it into the tmux session as environment variables:

    ```mermaid
    flowchart TD
        START[Launch isolated agent] --> TAKEN["Collect ports held by<br/>other live agents"]
        TAKEN --> ALLOC["allocate(base, taken, is_free)"]
        ALLOC --> PROBE{"Port free?<br/>(TCP bind probe)"}
        PROBE -->|no| NEXT[try next port]
        NEXT --> PROBE
        PROBE -->|yes| RESERVE["Persist port on agent"]
        RESERVE --> ENV["tmux new-session -e PORT=41000<br/>-e KAIJU_AGENT_PORT=41000"]
        ENV --> RUN["Agent's npm run dev<br/>picks up PORT"]
    ```

    The selection logic is, again, a tiny **pure function** with the probe injected — so it's testable without ever opening a socket:

    ```rust
    /// Pure: pick the first port at or above `base` that is neither already
    /// reserved (`taken`) nor reported busy by `is_free`.
    fn allocate(base: u16, taken: &HashSet<u16>, is_free: impl Fn(u16) -> bool) -> Option<u16> {
        (0..MAX_ATTEMPTS)
            .filter_map(|offset| base.checked_add(offset))
            .find(|port| !taken.contains(port) && is_free(*port))
    }
    ```

    Two failure modes guarded in one line: `taken` (a port another live agent reserved this run, before its server is even up) and `is_free` (a real TCP bind probe for anything else listening). `checked_add` means a saturated high range can't overflow `u16` and wrap around to port 0.

    The real wiring is trivial on top of it:

    ```rust
    fn is_free(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    pub fn allocate_port(taken: &HashSet<u16>) -> Option<u16> {
        let base = std::env::var("KAIJU_PORT_BASE")
            .ok().and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_BASE);   // 41000 — well clear of 3000/5173/8080
        allocate(base, taken, is_free)
    }
    ```

    Base port `41000` is chosen high on purpose, to stay clear of the common dev ports you'd run by hand. The reserved port is **persisted** on the agent and re-injected on resume, and the dashboard links straight to `localhost:<port>` so you can click through to the right preview.

    ### The honest caveat

    This is **best-effort by design**, and it's worth being upfront about why. Injecting `PORT` helps the servers that read it — Next.js, Create React App, most Express apps. **Vite reads `--port`, not `PORT`**, so a Vite project would need to reference `$KAIJU_AGENT_PORT` explicitly. I also only inject for *isolated* agents: a solo agent's `npm run dev` staying on the familiar `:3000` is a feature, not a bug. The goal isn't to guarantee every dev server relocates — it's to make sure parallel agents *can* coexist without me refereeing.

    ---

    ## The thread connecting both fixes

    Look at the two changes side by side and the same shape appears:

    | | Status detection | Port allocation |
    |---|---|---|
    | **Pure core** | `resolve_status(...)` | `allocate(base, taken, is_free)` |
    | **Injected dependency** | idle time (a number) | `is_free` probe (a closure) |
    | **Messy IO shell** | `poll_once` → tmux capture | `allocate_port` → `TcpListener::bind` |
    | **Tests** | a status truth table | a port truth table |

    Neither fix is clever. Both are small. But both put the *decision* somewhere I can prove it correct, and left only the unavoidable IO in the part I can't. That's the whole trick to making an orchestrator you actually trust: **the logic is pure, the world is a parameter.**

    The daemon now ships behind a hard gate I learned to respect the hard way — `cargo fmt`, `cargo clippy`, `cargo test`, and the dashboard's JS tests all green before anything merges. (A skipped `fmt` once shipped to `main` and earned me exactly the "how sloppy" I deserved.)

    ---

    ## Takeaways

    1. **A status detector you don't believe is worse than none.** False positives don't just annoy — they train you to ignore the signal entirely. Gate alerts on *confirmed* transitions.
    2. **Reuse the signal you already have.** I didn't add a new timer for the wait-confirm gate; I reused the idle clock the stuck-detector already computed. One number, two jobs.
    3. **Inject the world.** A TCP probe behind a closure and elapsed-time behind an `i64` turned two IO-bound problems into pure functions with truth-table tests.
    4. **Be honest about best-effort.** `PORT` injection doesn't cover Vite. Saying so in the docs is cheaper than a confused bug report later.

    The kaiju is still loose in my terminal. But now when the dashboard says an agent needs me, it actually does — and when three of them spin up dev servers, they each get their own door.

    ---

    *Kaiju is a personal project: a Rust workspace (`kaiju-core` / `kaiju-adapters` / `kaiju-daemon` / `kaiju-cli`) for orchestrating local AI coding agents, with a LAN-only daemon and a live dashboard. Built with KISS, SRP, and "push the logic into pure functions" as house rules.*
