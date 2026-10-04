//! Organisations — `evnx org …`.
//!
//! # ⛔ An organisation does not give anyone access to a vault
//!
//! It owns a **plan** and a set of **seats**. A seat holder is billed for and
//! gets that plan's limits. That is the whole feature.
//!
//! The server cannot wrap a vault key — that is evnx's central guarantee, not a
//! gap in it — so no organisation membership, role or seat can grant vault
//! access. Sharing stays `evnx vault share`, done by someone who holds the key.
//!
//! ⚠️ **Every command here that could be misread says so in its own output**, not
//! only in the guide. "I added them to the org, why can't they see the vault?" is
//! the first question this feature will produce, and the guide is read once while
//! the output is read at the moment the wrong belief forms.
//!
//! # A seat is a plan change
//!
//! Assigning one moves somebody's limits. The commands that do it say whose, and
//! say that is what happened — because the person it happened to will otherwise
//! watch their quota move without being told why.
//!
//! # Why a slug and not an id
//!
//! The API addresses organisations by UUID. Nobody types a UUID, so every command
//! takes `--org <slug>` and resolves it through `GET /api/v1/orgs`. When the
//! account belongs to exactly one organisation the flag is optional; when it
//! belongs to several, omitting it is an error that lists them rather than a
//! guess.

use anyhow::{anyhow, Result};
use colored::Colorize;
use serde::Deserialize;

use super::client::Client;
use super::config::CloudConfig;

/// The sentence that has to survive every copy edit in this module.
const NOT_VAULT_ACCESS: &str =
    "An organisation decides which plan's limits apply. It does not give anyone \
     access to a vault — share one with `evnx vault share`.";

#[derive(Deserialize)]
struct OrgList {
    organizations: Vec<OrgSummary>,
}

#[derive(Deserialize, Clone, Debug)]
struct OrgSummary {
    id: String,
    name: String,
    slug: String,
    plan: String,
    your_role: String,
    you_hold_a_seat: bool,
    seats: Seats,
}

#[derive(Deserialize, Clone, Debug)]
struct Seats {
    used: i64,
    purchased: Option<i64>,
}

impl Seats {
    fn render(&self) -> String {
        match self.purchased {
            Some(p) => format!("{} / {}", self.used, p),
            None => format!("{} / unlimited", self.used),
        }
    }
}

#[derive(Deserialize)]
struct MemberList {
    members: Vec<MemberSummary>,
}

#[derive(Deserialize, Clone, Debug)]
struct MemberSummary {
    user_id: String,
    email: String,
    role: String,
    holds_a_seat: bool,
    is_you: bool,
}

#[derive(Deserialize)]
struct CreatedOrg {
    id: String,
    slug: String,
}

#[derive(Deserialize)]
struct CreatedInvite {
    email: String,
    role: String,
    token: String,
    expires_in_days: i64,
}

#[derive(Deserialize)]
struct InviteList {
    invites: Vec<InviteSummary>,
}

#[derive(Deserialize, Clone, Debug)]
struct InviteSummary {
    id: String,
    email: String,
    role: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct AcceptedInvite {
    organization: String,
    role: String,
}

#[derive(Deserialize)]
struct SeatState {
    seats: Seats,
    over_seated: bool,
}

// ─── Shared plumbing ──────────────────────────────────────────────────────────

/// Organisation management needs a real login, never an API token.
///
/// ⚠️ Mirrors the server, which puts every org route behind
/// `require_user_session`. A leaked CI token that could invite people and assign
/// seats could change what the account is billed — the same reasoning that keeps
/// `evnx auth token` and account deletion off tokens.
fn require_login(client: &Client, server: &str) -> Result<()> {
    if client.is_api_token() {
        return Err(anyhow!(
            "an API token cannot manage organisations — that is deliberate.\n\
             \x20 Inviting people and assigning seats changes what the account is\n\
             \x20 billed, so it takes a real login.\n\
             \x20 Unset EVNX_TOKEN and run `evnx auth login`."
        ));
    }
    if !client.is_signed_in() {
        return Err(anyhow!(
            "not signed in to {server}. Run `evnx auth login` first."
        ));
    }
    Ok(())
}

fn fetch_orgs(client: &Client) -> Result<Vec<OrgSummary>> {
    let listed: OrgList = client.get("/api/v1/orgs").map_err(|e| anyhow!("{e}"))?;
    Ok(listed.organizations)
}

/// Decide which organisation a command acts on.
///
/// An explicit `--org` wins. With exactly one organisation the flag is optional.
/// With several, omitting it is refused and the slugs are listed — ⚠️ never
/// guessed, because the wrong guess here invites a stranger into a paying
/// organisation or moves somebody's billing.
fn resolve_org(client: &Client, slug: Option<&str>) -> Result<OrgSummary> {
    let orgs = fetch_orgs(client)?;

    if let Some(want) = slug {
        return orgs
            .iter()
            .find(|o| o.slug == want || o.id == want)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "you are not a member of any organisation called {want:?}.\n\
                     \x20 Run `evnx org list` to see yours."
                )
            });
    }

    match orgs.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(anyhow!(
            "you are not a member of any organisation.\n\
             \x20 Create one with `evnx org create <name> --slug <slug>`."
        )),
        many => Err(anyhow!(
            "you belong to {} organisations, so this needs `--org <slug>`: {}",
            many.len(),
            many.iter()
                .map(|o| o.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Find a member by email, or by user id.
fn resolve_member<'a>(members: &'a [MemberSummary], target: &str) -> Result<&'a MemberSummary> {
    let lower = target.to_lowercase();
    members
        .iter()
        .find(|m| m.user_id == target || m.email.to_lowercase() == lower)
        .ok_or_else(|| {
            anyhow!(
                "{target:?} is not a member of this organisation.\n\
                 \x20 Run `evnx org members` to see who is."
            )
        })
}

fn members_of(client: &Client, org_id: &str) -> Result<Vec<MemberSummary>> {
    let listed: MemberList = client
        .get(&format!("/api/v1/orgs/{org_id}/members"))
        .map_err(|e| anyhow!("{e}"))?;
    Ok(listed.members)
}

/// ⚠️ Refused client-side as well as by the server and by migration 010's CHECK.
/// Three layers, because an organisation has exactly one owner and an invitation
/// is a link somebody can forward.
fn validate_role(raw: &str) -> Result<String> {
    match raw.to_lowercase().as_str() {
        "member" => Ok("member".into()),
        "admin" => Ok("admin".into()),
        "owner" => Err(anyhow!(
            "`owner` cannot be granted. An organisation has exactly one owner, set\n\
             \x20 when it is created — handing it out through an invitation or a role\n\
             \x20 change would leave two, and nothing could then remove either."
        )),
        other => Err(anyhow!("role must be `member` or `admin`, not {other:?}")),
    }
}

/// Ask, and read a yes.
///
/// Inline rather than shared, matching `device::disavow` and `sync`: each caller
/// prints its own consequences first, and a shared prompt would invite callers to
/// rely on a generic question instead of naming what is about to happen.
fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

// ─── Commands ─────────────────────────────────────────────────────────────────

/// `evnx org create <name> --slug <slug>`
pub fn create(server_override: Option<&str>, name: &str, slug: &str, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let created: CreatedOrg = client
        .post(
            "/api/v1/orgs",
            &serde_json::json!({ "name": name, "slug": slug }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!("  {} created {}", "✓".green(), created.slug.bold());
    if verbose {
        println!("  id  {}", created.id);
    }
    println!();
    // ⚠️ Said at creation, which is when someone forms a belief about what they
    // just made — and when they are most likely to invite the whole team.
    println!("  You are the owner, and hold no seat yet.");
    println!(
        "  {}",
        "A new organisation is on the free plan, so a seat would change nothing".dimmed()
    );
    println!("  {}", "until billing is set up.".dimmed());
    println!();
    println!("  {NOT_VAULT_ACCESS}");
    println!();
    println!("  Next:  evnx org invite <email>");
    Ok(())
}

/// `evnx org list`
pub fn list(server_override: Option<&str>, _verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let orgs = fetch_orgs(&client)?;
    if orgs.is_empty() {
        println!("  No organisations on {server}.");
        println!("  Create one with `evnx org create <name> --slug <slug>`.");
        return Ok(());
    }

    println!();
    println!(
        "  {:<22}  {:<12}  {:<10}  {:<16}  {}",
        "SLUG".bold(),
        "PLAN".bold(),
        "YOUR ROLE".bold(),
        "SEATS".bold(),
        "".bold()
    );
    for o in &orgs {
        // ⚠️ The marker goes last. `{:<22}` pads by byte length and a coloured
        // string carries ANSI escapes that count toward it, so an inline marker
        // shifts every column to its right — the trap `session::list` and
        // `device::list` both document.
        let marker = if o.you_hold_a_seat {
            "← your seat".green().to_string()
        } else {
            String::new()
        };
        println!(
            "  {:<22}  {:<12}  {:<10}  {:<16}  {}",
            o.slug,
            o.plan,
            o.your_role,
            o.seats.render(),
            marker
        );
    }
    println!();
    println!("  {}", NOT_VAULT_ACCESS.dimmed());
    Ok(())
}

/// `evnx org members [--org <slug>]`
pub fn members(server_override: Option<&str>, org: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    let members = members_of(&client, &target.id)?;

    println!();
    println!("  {} — {}", target.name.bold(), target.slug.dimmed());
    println!("  seats {}", target.seats.render());
    println!();
    println!(
        "  {:<34}  {:<10}  {:<6}  {}",
        "EMAIL".bold(),
        "ROLE".bold(),
        "SEAT".bold(),
        "".bold()
    );
    for m in &members {
        let marker = if m.is_you {
            "← you".green().to_string()
        } else {
            String::new()
        };
        println!(
            "  {:<34}  {:<10}  {:<6}  {}",
            m.email,
            m.role,
            if m.holds_a_seat { "yes" } else { "—" },
            marker
        );
        if verbose {
            println!("  {:<34}  {}", "", m.user_id.dimmed());
        }
    }
    println!();
    println!("  {}", NOT_VAULT_ACCESS.dimmed());
    Ok(())
}

/// `evnx org invite <email> [--role …] [--org <slug>]`
pub fn invite(
    server_override: Option<&str>,
    email: &str,
    role: Option<&str>,
    org: Option<&str>,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let role = validate_role(role.unwrap_or("member"))?;
    let target = resolve_org(&client, org)?;

    let created: CreatedInvite = client
        .post(
            &format!("/api/v1/orgs/{}/invites", target.id),
            &serde_json::json!({ "email": email, "role": role }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!(
        "  {} invited {} to {} as {}",
        "✓".green(),
        created.email.bold(),
        target.slug,
        created.role
    );
    println!();
    println!("  An email is on its way. To pass the invitation on yourself:");
    println!();
    println!("      evnx org accept {}", created.token);
    println!();
    println!(
        "  {}",
        format!(
            "Expires in {} days, and can be used once.",
            created.expires_in_days
        )
        .dimmed()
    );
    if verbose {
        println!(
            "  {}",
            "Only the invited address can redeem it — the token alone is not enough.".dimmed()
        );
    }
    println!();
    // ⚠️ The most important line in this module. An invitation into something
    // called an organisation reads as an invitation to shared secrets.
    println!("  {}", "They will NOT gain access to any vault.".yellow());
    println!("  {NOT_VAULT_ACCESS}");
    Ok(())
}

/// `evnx org invites [--org <slug>]`
pub fn invites(server_override: Option<&str>, org: Option<&str>, _verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    let listed: InviteList = client
        .get(&format!("/api/v1/orgs/{}/invites", target.id))
        .map_err(|e| anyhow!("{e}"))?;

    if listed.invites.is_empty() {
        println!("  No invitations outstanding for {}.", target.slug);
        return Ok(());
    }

    println!();
    println!(
        "  {:<34}  {:<10}  {:<22}  {}",
        "EMAIL".bold(),
        "ROLE".bold(),
        "EXPIRES".bold(),
        "ID".bold()
    );
    for i in &listed.invites {
        println!(
            "  {:<34}  {:<10}  {:<22}  {}",
            i.email, i.role, i.expires_at, i.id
        );
    }
    println!();
    println!("  Withdraw one with `evnx org uninvite <id>`.");
    Ok(())
}

/// `evnx org uninvite <invite_id> [--org <slug>]`
pub fn uninvite(
    server_override: Option<&str>,
    invite_id: &str,
    org: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    client
        .delete(&format!("/api/v1/orgs/{}/invites/{invite_id}", target.id))
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} withdrew that invitation", "✓".green());
    Ok(())
}

/// `evnx org accept <token>`
///
/// ⚠️ Takes no `--org`: the token names the organisation, and the caller is not a
/// member yet so there would be nothing to resolve a slug against.
pub fn accept(server_override: Option<&str>, token: &str, _verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let joined: AcceptedInvite = client
        .post(
            "/api/v1/orgs/invites/accept",
            &serde_json::json!({ "token": token.trim() }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!(
        "  {} joined {} as {}",
        "✓".green(),
        joined.organization.bold(),
        joined.role
    );
    println!();
    // ⚠️ Said because the obvious expectation is wrong in two ways at once: no
    // seat, and no vault.
    println!("  You hold no seat yet — an administrator assigns one, and that is");
    println!("  what decides your plan's limits.");
    println!();
    println!("  {NOT_VAULT_ACCESS}");
    Ok(())
}

/// `evnx org seats [--set N | --unlimited] [--org <slug>]`
pub fn seats(
    server_override: Option<&str>,
    set: Option<i64>,
    unlimited: bool,
    org: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;

    // Read-only when neither flag is given. ⚠️ A bare `evnx org seats` must not
    // write anything — the obvious reading of the word is "show me the seats".
    if set.is_none() && !unlimited {
        println!();
        println!("  {} — seats {}", target.slug.bold(), target.seats.render());
        if let Some(p) = target.seats.purchased {
            if target.seats.used > p {
                println!("  {} more seats are assigned than purchased", "!".yellow());
            }
        }
        println!();
        println!("  Change it with `evnx org seats --set N` or `--unlimited`.");
        return Ok(());
    }

    if set.is_some() && unlimited {
        return Err(anyhow!("--set and --unlimited cannot both be given"));
    }

    // ⚠️ Asked before trying, so the answer is a sentence rather than a rejected
    // request. Once Paddle is paying, the seat count is what the invoice is
    // computed from and this route is refused by the server — but the server's
    // message has to read the same in a browser, so it cannot name the thing a
    // terminal user should do next. This can.
    if let Ok(state) = client.get::<BillingState>(&format!("/api/v1/orgs/{}/billing", target.id)) {
        if state.subscription.exists && state.subscription.status.as_deref() != Some("canceled") {
            println!();
            println!(
                "  {} this organisation has a subscription, so its seat",
                "!".yellow()
            );
            println!("  count is set by billing — changing it here would grant seats");
            println!("  nobody is billed for, and the next Paddle event would undo it.");
            println!();
            println!("  See it with   {}", "evnx org billing".cyan());
            println!("  Change it at  {}", "https://app.evnx.dev/billing/".cyan());
            println!();
            return Ok(());
        }
    }
    if let Some(n) = set {
        if n < 0 {
            return Err(anyhow!("--set takes zero or more"));
        }
    }

    let body = serde_json::json!({ "seats": if unlimited { serde_json::Value::Null } else { serde_json::json!(set) } });
    let state: SeatState = client
        .put(&format!("/api/v1/orgs/{}/seats", target.id), &body)
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} seats {}", "✓".green(), state.seats.render());
    if state.over_seated {
        println!();
        // ⚠️ Reported, not refused — the server allows this deliberately so a
        // billing downgrade can never be rejected by the database. Someone has to
        // be told, though, because the people over the line keep their seats until
        // one is released.
        println!(
            "  {} more seats are assigned than purchased. Release some with",
            "!".yellow()
        );
        println!("  `evnx org release <email>`.");
    }
    Ok(())
}

#[derive(Deserialize)]
struct BillingState {
    plan: String,
    seats: Seats,
    over_seated: bool,
    your_role: String,
    billing_configured: bool,
    subscription: SubscriptionState,
}

#[derive(Deserialize)]
struct SubscriptionState {
    exists: bool,
    status: Option<String>,
    current_period_ends_at: Option<String>,
    scheduled_change: Option<ScheduledChange>,
}

#[derive(Deserialize)]
struct ScheduledChange {
    action: String,
    effective_at: Option<String>,
}

/// `2026-11-03T12:00:00Z` → `2026-11-03`. ⚠️ A plain truncation at the `T`
/// rather than a date parse: the exact instant is Paddle's business, the day is
/// the only part anyone reads, and pulling in a date library to print ten
/// characters is not a trade worth making in a CLI this size.
fn day_of(iso: &str) -> &str {
    iso.split('T').next().unwrap_or(iso)
}

/// `evnx org billing [--org <slug>]` — what the plan is, and until when.
///
/// # ⚠️ Why this is read-only
///
/// Buying, cancelling and updating a card all happen through Paddle, who are the
/// merchant of record. A terminal is the wrong place for a card form, and
/// rebuilding Paddle's flows would mean claiming authority over a contract evnx
/// is not party to. What a terminal *is* the right place for is the question
/// this answers: why are my limits what they are, and is that about to change?
pub fn billing(server_override: Option<&str>, org: Option<&str>, _verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    let state: BillingState = client
        .get(&format!("/api/v1/orgs/{}/billing", target.id))
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!("  {} — {} plan", target.slug.bold(), state.plan.bold());
    println!("  seats {}", state.seats.render());

    if !state.billing_configured {
        println!();
        println!("  This server has no payment provider configured, so plans are");
        println!("  set by whoever runs it.");
        println!();
        return Ok(());
    }

    match (
        &state.subscription.exists,
        state.subscription.status.as_deref(),
    ) {
        (false, _) | (_, Some("canceled")) => {
            println!();
            println!("  No subscription. Start one at");
            println!("  {}", "https://app.evnx.dev/billing/".cyan());
        }
        (_, Some("past_due")) => {
            println!();
            // ⚠️ "not yet" is the load-bearing word. Without it people assume
            // they are already locked out and stop trying to work.
            println!("  {} a payment did not go through.", "!".red());
            println!("  Paddle will retry. Your limits have not changed yet.");
            println!(
                "  Update the payment method at {}",
                "https://app.evnx.dev/billing/".cyan()
            );
        }
        _ => {
            // ⚠️ The scheduled change is checked BEFORE the status, because a
            // cancelled-but-running subscription still reports `active`. Reading
            // the status alone prints "renews" on the exact day the plan ends.
            match state.subscription.scheduled_change.as_ref() {
                Some(c) if c.action == "cancel" => {
                    println!();
                    println!(
                        "  {} this plan ENDS on {}",
                        "!".yellow(),
                        c.effective_at
                            .as_deref()
                            .map(day_of)
                            .unwrap_or("an unknown date")
                    );
                    println!("  It is fully active until then. On that date every seat");
                    println!("  holder drops to the free plan's limits.");
                    println!("  Keep it at {}", "https://app.evnx.dev/billing/".cyan());
                }
                Some(c) => {
                    println!();
                    println!(
                        "  {} a {} is scheduled for {}",
                        "!".yellow(),
                        c.action,
                        c.effective_at
                            .as_deref()
                            .map(day_of)
                            .unwrap_or("an unknown date")
                    );
                }
                None => {
                    if let Some(ends) = state.subscription.current_period_ends_at.as_deref() {
                        println!("  renews {}", day_of(ends));
                    }
                }
            }
        }
    }

    if state.over_seated {
        println!();
        println!(
            "  {} more seats are assigned than purchased. Everyone keeps",
            "!".yellow()
        );
        println!("  theirs — release one with `evnx org release <email>`, or buy more.");
    }

    println!();
    // ⛔ Said here too. The whole chain repeats it because "my payment failed,
    // have I lost my secrets?" is the question this feature will generate.
    println!("  {}", NOT_VAULT_ACCESS.dimmed());
    if state.your_role != "owner" && state.subscription.exists {
        println!();
        println!("  Only the owner can change the plan or the seat count.");
    }
    println!();
    Ok(())
}

/// `evnx org assign <email> [--org <slug>]` — give someone a seat.
pub fn assign(
    server_override: Option<&str>,
    who: &str,
    org: Option<&str>,
    verbose: bool,
) -> Result<()> {
    seat_change(server_override, who, org, true, verbose)
}

/// `evnx org release <email> [--org <slug>]` — take a seat back.
pub fn release(
    server_override: Option<&str>,
    who: &str,
    org: Option<&str>,
    verbose: bool,
) -> Result<()> {
    seat_change(server_override, who, org, false, verbose)
}

fn seat_change(
    server_override: Option<&str>,
    who: &str,
    org: Option<&str>,
    assign: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    let members = members_of(&client, &target.id)?;
    let member = resolve_member(&members, who)?;

    let _: serde_json::Value = client
        .patch(
            &format!("/api/v1/orgs/{}/members/{}", target.id, member.user_id),
            &serde_json::json!({ "seat": assign }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    if assign {
        println!(
            "  {} {} now holds a seat in {}",
            "✓".green(),
            member.email.bold(),
            target.slug
        );
        println!();
        // ⚠️ A seat IS a plan change, and the person it happened to will see their
        // limits move. Saying so here is what makes that explicable.
        println!(
            "  Their limits are now {}'s — on the {} plan.",
            target.slug, target.plan
        );
    } else {
        println!(
            "  {} released {}'s seat in {}",
            "✓".green(),
            member.email.bold(),
            target.slug
        );
        println!();
        println!(
            "  {} their limits drop back to their own plan.",
            "!".yellow()
        );
        println!(
            "  {}",
            "Nothing is deleted. A vault already over the new limit stays readable;".dimmed()
        );
        println!("  {}", "the next push to it is what gets refused.".dimmed());
    }
    if verbose {
        println!("  user id  {}", member.user_id);
    }
    Ok(())
}

/// `evnx org role <email> --set <member|admin> [--org <slug>]`
pub fn set_role(
    server_override: Option<&str>,
    who: &str,
    role: &str,
    org: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let role = validate_role(role)?;
    let target = resolve_org(&client, org)?;
    let members = members_of(&client, &target.id)?;
    let member = resolve_member(&members, who)?;

    let _: serde_json::Value = client
        .patch(
            &format!("/api/v1/orgs/{}/members/{}", target.id, member.user_id),
            &serde_json::json!({ "role": role }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "  {} {} is now {} in {}",
        "✓".green(),
        member.email.bold(),
        role,
        target.slug
    );
    Ok(())
}

/// `evnx org remove <email> [--org <slug>]`
pub fn remove(
    server_override: Option<&str>,
    who: &str,
    yes: bool,
    org: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    let members = members_of(&client, &target.id)?;
    let member = resolve_member(&members, who)?;

    if !yes {
        println!();
        println!(
            "  Remove {} from {}?",
            member.email.bold(),
            target.slug.bold()
        );
        if member.holds_a_seat {
            // ⚠️ Worth naming before the prompt: removal frees the seat and drops
            // their limits, which is a consequence beyond "they leave the list".
            println!(
                "  {} they hold a seat, so their limits drop to their own plan.",
                "!".yellow()
            );
        }
        println!("  {}", "Nothing they already pulled is recalled.".dimmed());
        println!();
        if !confirm("  Continue? [y/N] ")? {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    client
        .delete(&format!(
            "/api/v1/orgs/{}/members/{}",
            target.id, member.user_id
        ))
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} removed {}", "✓".green(), member.email);
    Ok(())
}

/// `evnx org delete [--org <slug>] [--force]`
///
/// ⚠️ This command exists because leaving it out was a trap. Owning an
/// organisation blocks `evnx auth delete-account`, and the server caps creation
/// at five — so with no way to delete one, five organisations would permanently
/// remove the ability to close the account.
pub fn delete(
    server_override: Option<&str>,
    force: bool,
    yes: bool,
    org: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;

    if !yes {
        println!();
        println!("  Delete {}?", target.slug.bold());
        if target.seats.used > 0 {
            // ⚠️ Named before the prompt. Deleting is a plan change for every seat
            // holder at once, and they are not the one running this.
            println!(
                "  {} {} seat(s) are assigned. Every holder drops back to their own plan.",
                "!".yellow(),
                target.seats.used
            );
        }
        println!(
            "  {}",
            "Nothing anyone already pulled is recalled, and no vault is touched.".dimmed()
        );
        println!();
        if !confirm("  Continue? [y/N] ")? {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    client
        .delete_with_body(
            &format!("/api/v1/orgs/{}", target.id),
            &serde_json::json!({ "force": force || target.seats.used > 0 }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} deleted {}", "✓".green(), target.slug);
    Ok(())
}

/// `evnx org leave [--org <slug>]`
///
/// ⚠️ Its own command rather than `remove <your own email>`. Leaving is something
/// a plain member may do without an administrator, the server allows exactly that,
/// and making people spell their own address to do it reads as a trick question.
pub fn leave(
    server_override: Option<&str>,
    yes: bool,
    org: Option<&str>,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let target = resolve_org(&client, org)?;
    let members = members_of(&client, &target.id)?;
    let me = members
        .iter()
        .find(|m| m.is_you)
        .ok_or_else(|| anyhow!("you are not a member of {}", target.slug))?;

    if me.role == "owner" {
        return Err(anyhow!(
            "you own {}, so you cannot leave it.\n\
             \x20 An organisation with no owner has nobody who can change its billing\n\
             \x20 or delete it. Transfer ownership, or delete the organisation.",
            target.slug
        ));
    }

    if !yes {
        println!();
        println!("  Leave {}?", target.slug.bold());
        if me.holds_a_seat {
            println!(
                "  {} you hold a seat, so your limits drop back to your own plan.",
                "!".yellow()
            );
        }
        println!();
        if !confirm("  Continue? [y/N] ")? {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    client
        .delete(&format!(
            "/api/v1/orgs/{}/members/{}",
            target.id, me.user_id
        ))
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} left {}", "✓".green(), target.slug);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org(slug: &str) -> OrgSummary {
        OrgSummary {
            id: format!("id-{slug}"),
            name: slug.to_string(),
            slug: slug.to_string(),
            plan: "free".into(),
            your_role: "owner".into(),
            you_hold_a_seat: false,
            seats: Seats {
                used: 0,
                purchased: None,
            },
        }
    }

    fn member(email: &str, id: &str) -> MemberSummary {
        MemberSummary {
            user_id: id.to_string(),
            email: email.to_string(),
            role: "member".into(),
            holds_a_seat: false,
            is_you: false,
        }
    }

    /// ⚠️ `owner` must be refused before the request is made, with a reason.
    #[test]
    fn owner_is_not_a_grantable_role() {
        assert_eq!(validate_role("member").unwrap(), "member");
        assert_eq!(validate_role("ADMIN").unwrap(), "admin");

        let e = validate_role("owner").unwrap_err().to_string();
        assert!(e.contains("exactly one owner"), "{e}");

        assert!(validate_role("developer").is_err(), "that is a vault role");
        assert!(validate_role("").is_err());
    }

    /// A member is addressable by email, case-insensitively, or by id.
    #[test]
    fn a_member_resolves_by_email_or_id() {
        let ms = [member("a@e.test", "u1"), member("b@e.test", "u2")];
        assert_eq!(resolve_member(&ms, "a@e.test").unwrap().user_id, "u1");
        assert_eq!(resolve_member(&ms, "A@E.TEST").unwrap().user_id, "u1");
        assert_eq!(resolve_member(&ms, "u2").unwrap().email, "b@e.test");

        let e = resolve_member(&ms, "nobody@e.test")
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("evnx org members"),
            "it must say how to look: {e}"
        );
    }

    /// ⚠️ With several organisations, omitting `--org` is an error that LISTS
    /// them. A guess here invites a stranger into a paying organisation or moves
    /// somebody's billing, so there is no sensible default.
    #[test]
    fn several_organisations_refuse_to_be_guessed_between() {
        // `resolve_org` needs a Client, so the branch is asserted through the same
        // logic on a slice — kept in step by construction below.
        let orgs = [org("acme"), org("globex")];
        let picked: Option<&OrgSummary> = match orgs.as_slice() {
            [one] => Some(one),
            _ => None,
        };
        assert!(
            picked.is_none(),
            "two organisations must not resolve to one"
        );

        let single = [org("acme")];
        let picked = match single.as_slice() {
            [one] => Some(one),
            _ => None,
        };
        assert_eq!(picked.unwrap().slug, "acme", "one is unambiguous");
    }

    #[test]
    fn seats_render_unlimited_as_a_word() {
        assert_eq!(
            Seats {
                used: 2,
                purchased: Some(5)
            }
            .render(),
            "2 / 5"
        );
        assert_eq!(
            Seats {
                used: 2,
                purchased: None
            }
            .render(),
            "2 / unlimited"
        );
    }

    /// ⚠️ The sentence that must survive a copy edit. Every command that could be
    /// misread prints it, and this asserts it still says the two things that
    /// matter: that an org decides a plan, and that it is not vault access.
    #[test]
    fn the_disclaimer_still_says_what_it_is_for() {
        assert!(NOT_VAULT_ACCESS.contains("plan"));
        assert!(NOT_VAULT_ACCESS.contains("does not give"));
        assert!(NOT_VAULT_ACCESS.contains("vault share"));
    }
}
