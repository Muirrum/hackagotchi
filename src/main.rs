#![feature(decl_macro)]
#![feature(proc_macro_hygiene)]
#![feature(try_trait)]
#![recursion_limit = "512"]
use crossbeam_channel::Sender;
use rocket::request::LenientForm;
use rocket::tokio;
use rocket::{post, routes, FromForm, State};
use rocket_contrib::json::Json;
use rusoto_dynamodb::{AttributeValue, DynamoDb, DynamoDbClient};
use serde_json::{json, Value};
use std::convert::TryInto;

mod banker;
mod config;
mod hacksteader;
mod market;

use hacksteader::{Gotchi, Hacksteader, Possessed, Possession};

fn dyn_db() -> DynamoDbClient {
    DynamoDbClient::new(rusoto_core::Region::UsEast1)
}

lazy_static::lazy_static! {
    pub static ref TOKEN: String = std::env::var("TOKEN").unwrap();
    pub static ref ID: String = std::env::var("ID").unwrap();
    pub static ref APP_ID: String = std::env::var("APP_ID").unwrap();
    pub static ref URL: String = std::env::var("URL").unwrap();
    pub static ref HACKSTEAD_PRICE: u64 = std::env::var("HACKSTEAD_PRICE").unwrap().parse().unwrap();
}

fn mrkdwn<S: std::string::ToString>(txt: S) -> Value {
    json!({
        "type": "mrkdwn",
        "text": txt.to_string(),
    })
}
fn plain_text<S: std::string::ToString>(txt: S) -> Value {
    json!({
        "type": "plain_text",
        "text": txt.to_string(),
    })
}
fn comment<S: ToString>(txt: S) -> Value {
    json!({
        "type": "context",
        "elements": [
            mrkdwn(txt)
        ]
    })
}
fn emojify<S: ToString>(txt: S) -> String {
    format!(":{}:", txt.to_string().replace(" ", "_"))
}
fn filify<S: ToString>(txt: S) -> String {
    txt.to_string().to_lowercase().replace(" ", "_")
}

async fn dm_blocks(user_id: String, blocks: Vec<Value>) -> Result<(), String> {
    let o = json!({
        "channel": user_id,
        "token": *TOKEN,
        "blocks": blocks
    });

    println!("{}", serde_json::to_string_pretty(&o).unwrap());

    // TODO: use response
    let client = reqwest::Client::new();
    client
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(&*TOKEN)
        .json(&o)
        .send()
        .await
        .map_err(|e| format!("couldn't dm {}: {}", user_id, e))?;

    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PossessionPage {
    possession: Possession,
    interactivity: Interactivity,
    credentials: Credentials,
}

impl PossessionPage {
    fn modal(self, trigger_id: String, page_json: String) -> Modal {
        Modal {
            callback_id: self.callback_id(),
            blocks: self.blocks(),
            submit: dbg!(self.submit()),
            title: self.possession.nickname().to_string(),
            method: "open".to_string(),
            trigger_id: trigger_id,
            private_metadata: page_json,
        }
    }

    fn modal_update(self, trigger_id: String, page_json: String, view_id: String) -> ModalUpdate {
        ModalUpdate {
            callback_id: self.callback_id(),
            blocks: self.blocks(),
            submit: self.submit(),
            title: self.possession.nickname().to_string(),
            private_metadata: page_json,
            trigger_id,
            view_id,
            hash: None,
        }
    }

    fn submit(&self) -> Option<String> {
        if let Some(sale) = dbg!(dbg!(self.possession.sale.as_ref())
            .filter(|_| self.interactivity.market(self.credentials)))
        {
            match self.credentials {
                Credentials::Owner => return Some("Take off Market".to_string()),
                Credentials::Hacksteader => return Some(format!("Buy for {} gp", sale.price)),
                _ => {}
            }
        }
        None
    }

    fn callback_id(&self) -> String {
        if self
            .possession
            .sale
            .as_ref()
            .filter(|_| self.interactivity.market(self.credentials))
            .is_some()
        {
            match self.credentials {
                Credentials::Owner => return "sale_removal".to_string(),
                Credentials::Hacksteader => return "sale_complete".to_string(),
                _ => {}
            }
        }
        "possession_page_".to_string() + self.interactivity.id()
    }

    fn blocks(&self) -> Vec<Value> {
        // TODO: with_capacity optimization
        let mut blocks: Vec<Value> = Vec::new();
        let Self {
            possession,
            interactivity,
            credentials,
        } = self;

        let actions = |prefix: &str, buttons: &[(&str, Option<Value>)]| -> Value {
            match interactivity {
                Interactivity::Write => json!({
                    "type": "actions",
                    "elements": buttons.iter().map(|(action, value)| {
                        let mut o = json!({
                            "type": "button",
                            "text": plain_text(action),
                            "action_id": format!("{}_{}", prefix, action.to_lowercase()),
                        });
                        if let Some(v) = value {
                            o.as_object_mut().unwrap().insert("value".to_string(), v.clone());
                        }
                        o
                    }).collect::<Vec<_>>()
                }),
                _ => comment("This page is read only."),
            }
        };

        if self.possession.kind.is_gotchi() {
            blocks.push(actions(
                "gotchi",
                &[("Nickname", Some(json!(possession.nickname())))],
            ));
        }

        let mut text_fields = vec![
            ("species", possession.name.clone()),
            (
                "owner log",
                possession
                    .ownership_log
                    .iter()
                    .map(|o| format!("[{}]<@{}>", o.acquisition, o.id))
                    .collect::<Vec<_>>()
                    .join(" -> ")
                    .to_string(),
            ),
        ];

        if let Some(g) = possession.kind.gotchi() {
            text_fields.push(("base happiness", g.base_happiness.to_string()));
        }

        let text = text_fields
            .iter()
            .map(|(l, r)| format!("*{}:* _{}_", l, r))
            .collect::<Vec<_>>()
            .join("\n");

        blocks.push(json!({
            "type": "section",
            "text": mrkdwn(text),
            "accessory": {
                "type": "image",
                "image_url": format!(
                    "http://{}/gotchi/img/{}/{}.png",
                    *URL,
                    format!("{:?}", possession.kind.category()).to_lowercase(),
                    filify(&possession.name)
                ),
                "alt_text": "hackagotchi img",
            }
        }));

        blocks.push(actions("possession", &[("Give", None), ("Sell", None)]));

        if let Some(g) = possession.kind.gotchi() {
            blocks.push(comment(format!(
                "*Lifetime GP harvested: {}*",
                g.harvest_log.iter().map(|x| x.harvested).sum::<u64>(),
            )));

            for owner in g.harvest_log.iter().rev() {
                blocks.push(comment(format!(
                    "{}gp harvested for <@{}>",
                    owner.harvested, owner.id
                )));
            }
        }

        if let (Credentials::None, true) = (credentials, interactivity.market(*credentials)) {
            blocks.push(json!({ "type": "divider" }));
            blocks.push(comment(
                "In order to buy this, you have to have a \
                <slack://app?team=T0266FRGM&id={}&tab=home|hackstead>.",
            ));
        }

        blocks
    }
}

async fn update_user_home_tab(user_id: String) -> Result<(), String> {
    update_home_tab(
        Hacksteader::from_db(&dyn_db(), user_id.clone()).await.ok(),
        user_id.clone(),
    )
    .await
}
async fn update_home_tab(hs: Option<Hacksteader>, user_id: String) -> Result<(), String> {
    let o = json!({
        "user_id": user_id,
        "view": {
            "type": "home",
            "blocks": hacksteader_greeting_blocks(hs, Interactivity::Write, Credentials::Owner),
        }
    });

    println!("home screen: {}", serde_json::to_string_pretty(&o).unwrap());

    let client = reqwest::Client::new();
    client
        .post("https://slack.com/api/views.publish")
        .bearer_auth(&*TOKEN)
        .json(&o)
        .send()
        .await
        .map_err(|e| format!("couldn't publish home tab view: {}", e))?;

    Ok(())
}

fn progress_bar(size: usize, progress_ratio: f32) -> String {
    format!(
        "`\u{2062}{}\u{2062}`",
        (0..size)
            .map(|i| {
                if (i as f32 / size as f32) < progress_ratio {
                    '\u{2588}'
                } else {
                    ' '
                }
            })
            .collect::<String>()
    )
}

fn hackstead_blocks(
    hs: Hacksteader,
    interactivity: Interactivity,
    credentials: Credentials,
) -> Vec<Value> {
    use humantime::format_duration;
    use std::time::SystemTime;

    // TODO: with_capacity optimization
    let mut blocks: Vec<Value> = Vec::new();

    let hs_adv = hs.profile.current_advancement();
    let next_hs_adv = hs.profile.next_advancement();

    blocks.push(json!({
        "type": "section",
        "text": mrkdwn(format!("*_<@{}>'s {}_* - _{}xp_", hs.user_id, hs_adv.achiever_title, hs.profile.xp)),
    }));

    blocks.push(comment(format!(
        "founded {} ago (roughly)",
        format_duration(SystemTime::now().duration_since(hs.profile.joined).unwrap()),
    )));
    if let Some(na) = next_hs_adv {
        blocks.push({
            let (xp_lo, xp_hi) = (hs_adv.xp, na.xp);
            let (have, need) = (hs.profile.xp - xp_lo, xp_hi - xp_lo);
            json!({
                "type": "section",
                "text": mrkdwn(format!(
                    "Next: *{}*\n{}  {}xp to go\n_{}_",
                    na.title,
                    progress_bar(50, have as f32 / need as f32),
                    need - have,
                    na.description
                )),
            })
        });
    }
    blocks.push(comment(format!("Last Advancement: \"{}\"", hs_adv.title)));

    blocks.push(json!({ "type": "divider" }));
    for tile in hs.land.into_iter() {
        blocks.push(json!({
            "type": "section",
            "text": mrkdwn(match tile.plant.as_ref() {
                Some(p) => {
                    let mut s = String::new();
                    let ca = p.current_advancement();
                    s.push_str(&format!("*{}* - _{}_ - {}xp\n\n", p.name, ca.achiever_title, p.xp));
                    if let Some(na) = p.next_advancement() {
                        let (xp_lo, xp_hi) = (ca.xp, na.xp);
                        let (have, need) = (p.xp - xp_lo, xp_hi - xp_lo);
                        s.push_str(&format!(
                            "Next: *{}*\n{}  {}xp to go\n_{}_",
                            na.title,
                            progress_bar(35, have as f32/need as f32),
                            need - have,
                            na.description
                        ));
                    }
                    s
                },
                None => "*Empty Land*\n Opportunity Awaits!".to_string()
            }),
            "accessory": {
                "type": "image",
                "image_url": match tile.plant.as_ref() {
                    Some(p) => format!("http://{}/gotchi/img/plant/{}.png", *URL, filify(&p.name)),
                    None => format!("http://{}/gotchi/img/misc/dirt.png", *URL),
                },
                "alt_text": match tile.plant.as_ref() {
                    Some(p) => format!("A healthy, growing {}!", p.name),
                    None => "Land, waiting to be monopolized upon!".to_string(),
                }
            }
        }));
        match tile.plant {
            Some(p) => {
                let ca = p.current_advancement();
                blocks.push(comment(format!("Last Advancement: \"{}\"", ca.title)));

                if let Some(applicables) = Some(
                    hs
                        .inventory
                        .iter()
                        .filter_map(|x| x.kind.keepsake())
                        .filter(|x| x.plant_applicable.is_some())
                        .collect::<Vec<_>>()
                    )
                    .filter(|x| !x.is_empty())
                {
                    blocks.push(json!({
                        "type": "actions",
                        "elements": [{
                            "type": "button",
                            "text": plain_text("Apply Item"),
                            "style": "primary",
                            "value": serde_json::to_string(&(tile.id.to_simple().to_string(), applicables)).unwrap(),
                            "action_id": "item_apply_select",
                        }],
                    }))
                }
            }
            None => {
                let seeds: Vec<Possessed<hacksteader::Seed>> = hs
                    .inventory
                    .iter()
                    .cloned()
                    .filter_map(|p| p.try_into().ok())
                    .collect();

                blocks.push(if seeds.is_empty() {
                    comment(":seedling: No seeds! See if you can buy some on the /hackmarket")
                } else if let Interactivity::Write = interactivity {
                    json!({
                        "type": "actions",
                        "elements": [{
                            "type": "button",
                            "text": plain_text("Plant Seed"),
                            "style": "primary",
                            "value": serde_json::to_string(&(tile.id.to_simple().to_string(), seeds)).unwrap(),
                            "action_id": "seed_plant",
                        }],
                    })
                } else {
                    comment(":seedling: No planting seeds for you! This page is read only.")
                });
            }
        }
    }

    blocks.push(json!({ "type": "divider" }));

    if hs.inventory.len() > 0 {
        blocks.push(json!({
            "type": "section",
            "text": mrkdwn("*Inventory*"),
        }));

        for possession in hs.inventory.into_iter() {
            blocks.push(json!({
                "type": "section",
                "text": mrkdwn(format!("_{} {}_", emojify(&possession.name), possession.name)),
                "accessory": {
                    "type": "button",
                    "style": "primary",
                    "text": plain_text(&possession.name),
                    "value": serde_json::to_string(&PossessionPage {
                        possession,
                        interactivity,
                        credentials,
                    }).unwrap(),
                    "action_id": "possession_page",
                }
            }));
        }
    } else {
        blocks.push(comment("Your inventory is empty"));
    }

    if hs.gotchis.len() > 0 {
        blocks.push(json!({ "type": "divider" }));

        blocks.push(json!({
            "type": "section",
            "text": mrkdwn(match hs.gotchis.len() {
                1 => "*Your Hackagotchi*".into(),
                _ => format!("*Your {} Hackagotchi*", hs.gotchis.len())
            }),
        }));

        let total_happiness = hs
            .gotchis
            .iter()
            .map(|g| g.inner.base_happiness)
            .sum::<u64>();

        for gotchi in hs.gotchis.into_iter() {
            blocks.push(json!({
                "type": "section",
                "text": mrkdwn(format!("_{} ({}, {} happiness)_", emojify(&gotchi.name), gotchi.name, gotchi.inner.base_happiness)),
                "accessory": {
                    "type": "button",
                    "style": "primary",
                    "text": plain_text(&gotchi.inner.nickname),
                    "value": serde_json::to_string(&PossessionPage {
                        possession: gotchi.into_possession(),
                        interactivity,
                        credentials,
                    }).unwrap(),
                    "action_id": "possession_page",
                }
            }));
        }

        blocks.push(json!({
            "type": "section",
            "text": mrkdwn(format!("Total happiness: *{}*", total_happiness))
        }));
        blocks.push(comment(
            "The total happiness of all your gotchi is equivalent to the \
             amount of GP you'll get at the next Harvest.",
        ));
    }

    if let Interactivity::Read = interactivity {
        blocks.push(json!({ "type": "divider" }));

        blocks.push(comment(format!(
            "This is a read-only snapshot of <@{}>'s Hackagotchi Hackstead at a specific point in time. \
            You can manage your own Hackagotchi Hackstead in real time at your \
            <slack://app?team=T0266FRGM&id={}&tab=home|hackstead>.",
            hs.user_id,
            *APP_ID,
        )));
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!( { "blocks": blocks.clone() })).unwrap()
    );

    blocks
}

macro_rules! hacksteader_opening_blurb { ( $hackstead_cost:expr ) => { format!(
"
*Your Own Hackagotchi Hackstead!*

:corn: Grow your own Farmables which make Hackagotchi happier!
:sparkling_heart: Earn passive income by collecting adorable Hackagotchi!
:money_with_wings: Buy, sell and trade Farmables and Hackagotchi at an open market!

Hacksteading costs *{} GP*.
As a Hacksteader, you'll have a plot of land on which to grow your own Farmables which make Hackagotchi happier. \
Happier Hackagotchi generate more passive income! \
You can also buy, sell, and trade Farmables and Hackagotchi for GP on an open market space. \
",
$hackstead_cost
) } }

fn hackstead_explanation_blocks() -> Vec<Value> {
    vec![
        json!({
            "type": "section",
            "text": mrkdwn(hacksteader_opening_blurb!(*HACKSTEAD_PRICE)),
        }),
        json!({
            "type": "actions",
            "elements": [{
                "type": "button",
                "action_id": "hackstead_confirm",
                "style": "danger",
                "text": plain_text("Monopolize on Adorableness?"),
                "confirm": {
                    "style": "danger",
                    "title": plain_text("Do you have what it takes"),
                    "text": mrkdwn("to be a Hackagotchi Hacksteader?"),
                    "deny": plain_text("I'm short on GP"),
                    "confirm": plain_text("LET'S HACKSTEAD, FRED!"),
                }
            }]
        }),
    ]
}

/// Returns Slack JSON displaying someone's hackstead if they're
/// registered, if not, this command will greet them with an explanation
/// of what hacksteading is and how they can get a hackstead of their own.
fn hacksteader_greeting_blocks(
    hacksteader: Option<Hacksteader>,
    interactivity: Interactivity,
    creds: Credentials,
) -> Vec<Value> {
    let o = match hacksteader {
        Some(hs) => hackstead_blocks(hs, interactivity, creds),
        None => hackstead_explanation_blocks(),
    };

    println!("{}", serde_json::to_string_pretty(&o).unwrap());

    o
}

async fn hackmarket_blocks() -> Vec<Value> {
    let sales = market::market_search(&dyn_db(), hacksteader::Category::Gotchi)
        .await
        .unwrap();
    let mut blocks = Vec::with_capacity(sales.len() * 2 + 3);

    blocks.push(json!({ "type": "divider" }));
    blocks.push(json!({
        "type": "section",
        "fields": [mrkdwn("*Name*"), mrkdwn("*Seller*")],
        "accessory": {
            "type": "button",
            "style": "danger",
            "text": plain_text("Price"),
        }
    }));
    blocks.push(json!({ "type": "divider" }));

    for (sale, mut gotchi) in sales.into_iter() {
        blocks.push(json!({
            "type": "section",
            "fields": [
                plain_text(format!("{} {}", emojify(&sale.market_name), sale.market_name)),
                mrkdwn(format!("<@{}>", gotchi.steader))
            ],
            "accessory": {
                "type": "button",
                "style": "primary",
                "text": plain_text(format!("{}gp", sale.price)),
                "action_id": "possession_market_page",
                "value": serde_json::to_string({
                    gotchi.sale.replace(sale);
                    &gotchi
                }).unwrap(),
            }
        }));
        blocks.push(json!({ "type": "divider" }));
    }

    dbg!(blocks)
}

#[derive(FromForm, Debug)]
struct SlashCommand {
    token: String,
    team_id: String,
    team_domain: String,
    channel_id: String,
    channel_name: String,
    user_id: String,
    user_name: String,
    command: String,
    text: String,
    response_url: String,
}

#[post("/hackmarket", data = "<_slash_command>")]
async fn hackmarket<'a>(_slash_command: LenientForm<SlashCommand>) -> Json<Value> {
    Json(json!({
        "blocks": hackmarket_blocks().await,
        "response_type": "in_channel",
    }))
}

#[post("/hackstead", data = "<slash_command>")]
async fn hackstead<'a>(slash_command: LenientForm<SlashCommand>) -> Json<Value> {
    println!("{:#?}", slash_command);

    lazy_static::lazy_static! {
        static ref HACKSTEAD_REGEX: regex::Regex = regex::Regex::new(
            "(<@([A-z0-9]+)|(.+)>)?"
        ).unwrap();
    }

    let captures = HACKSTEAD_REGEX.captures(&slash_command.text);
    println!("captures: {:#?}", captures);
    let user = captures
        .and_then(|c| c.get(2).map(|x| x.as_str()))
        .unwrap_or(&slash_command.user_id);

    let hs = Hacksteader::from_db(&dyn_db(), user.to_string()).await;
    Json(json!({
        "blocks": hacksteader_greeting_blocks(
            hs.ok(),
            Interactivity::Read,
            Credentials::None
        ),
        "response_type": "in_channel",
    }))
}

#[derive(FromForm, Debug)]
struct ActionData {
    payload: String,
}
#[derive(serde::Deserialize, Debug)]
pub struct Interaction {
    trigger_id: String,
    actions: Vec<Action>,
    user: User,
    view: Option<View>,
}
#[derive(serde::Deserialize, Debug)]
pub struct View {
    private_metadata: String,
    callback_id: String,
    root_view_id: String,
}
#[derive(serde::Deserialize, Debug)]
pub struct User {
    id: String,
}
#[derive(serde::Deserialize, Debug)]
pub struct Action {
    action_id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    value: String,
}

#[derive(Default)]
pub struct Modal {
    method: String,
    trigger_id: String,
    callback_id: String,
    title: String,
    private_metadata: String,
    blocks: Vec<Value>,
    submit: Option<String>,
}

impl Modal {
    async fn launch(self) -> Result<Value, String> {
        let mut o = json!({
            "trigger_id": self.trigger_id,
            "view": {
                "type": "modal",
                "private_metadata": self.private_metadata,
                "callback_id": self.callback_id,
                "title": plain_text(self.title),
                "blocks": self.blocks
            }
        });

        if let Some(submit_msg) = dbg!(self.submit) {
            o["view"]
                .as_object_mut()
                .unwrap()
                .insert("submit".to_string(), plain_text(submit_msg));
        }

        let client = reqwest::Client::new();
        client
            .post(&format!("https://slack.com/api/views.{}", self.method))
            .bearer_auth(&*TOKEN)
            .json(&o)
            .send()
            .await
            .map_err(|e| format!("couldn't open modal: {}", e))?;

        println!("{}", serde_json::to_string_pretty(&o).unwrap());
        Ok(o)
    }
}

#[derive(Default)]
pub struct ModalUpdate {
    trigger_id: String,
    callback_id: String,
    title: String,
    private_metadata: String,
    hash: Option<String>,
    view_id: String,
    blocks: Vec<Value>,
    submit: Option<String>,
}

impl ModalUpdate {
    async fn launch(self) -> Result<Value, String> {
        let mut o = json!({
            "trigger_id": self.trigger_id,
            "view_id": self.view_id,
            "view": {
                "type": "modal",
                "private_metadata": self.private_metadata,
                "callback_id": self.callback_id,
                "title": plain_text(self.title),
                "blocks": self.blocks
            }
        });

        if let Some(hash) = self.hash {
            o.as_object_mut()
                .unwrap()
                .insert("hash".to_string(), json!(hash));
        }

        if let Some(submit_msg) = self.submit {
            o["view"]
                .as_object_mut()
                .unwrap()
                .insert("submit".to_string(), plain_text(submit_msg));
        }

        let client = reqwest::Client::new();
        client
            .post("https://slack.com/api/views.update")
            .bearer_auth(&*TOKEN)
            .json(&o)
            .send()
            .await
            .map_err(|e| format!("couldn't open modal: {}", e))?;

        println!("{}", serde_json::to_string_pretty(&o).unwrap());
        Ok(o)
    }
}

#[derive(rocket::Responder)]
pub enum ActionResponse {
    Json(Json<Value>),
    Ok(()),
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
enum Interactivity {
    Read,
    Write,
    Buy,
}
impl Interactivity {
    fn id(self) -> &'static str {
        use Interactivity::*;
        match self {
            Read => "static",
            Write => "dynamic",
            Buy => "market",
        }
    }
}
impl Interactivity {
    fn market(&self, creds: Credentials) -> bool {
        match self {
            Interactivity::Buy => true,
            Interactivity::Write => creds == Credentials::Owner,
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Credentials {
    Owner,
    Hacksteader,
    None,
}

#[post("/interact", data = "<action_data>")]
async fn action_endpoint(action_data: LenientForm<ActionData>) -> Result<ActionResponse, String> {
    let v = serde_json::from_str::<Value>(&action_data.payload).unwrap();
    println!("action data: {:#?}", v);

    if let Some("view_submission") = v.get("type").and_then(|t| t.as_str()) {
        println!("right type!");
        let view = v.get("view").and_then(|view| {
            let parsed_view = serde_json::from_value::<View>(view.clone()).ok()?;
            let page_json_str = &parsed_view.private_metadata;
            let page: Option<PossessionPage> = match serde_json::from_str(&page_json_str) {
                Ok(page) => Some(page),
                Err(e) => {
                    dbg!(format!("couldn't parse {}: {}", page_json_str, e));
                    None
                }
            };
            Some((
                parsed_view,
                page,
                v.get("trigger_id")?.as_str()?,
                view.get("state").and_then(|s| s.get("values").cloned())?,
                serde_json::from_value::<User>(v.get("user")?.clone()).ok()?,
            ))
        });
        if let Some((view, Some(mut page), trigger_id, values, user)) = view {
            println!("view state values: {:#?}", values);

            match dbg!(view.callback_id.as_str()) {
                "sale_removal" => {
                    println!("Revoking sale");
                    market::take_off_market(
                        &dyn_db(),
                        hacksteader::Category::Gotchi,
                        page.possession.id,
                    )
                    .await?;

                    return Ok(ActionResponse::Json(Json(json!({
                        "response_action": "clear",
                    }))));
                }
                "sale_complete" => {
                    println!("Completing sale!");

                    if let Some(sale) = page.possession.sale.as_ref() {
                        banker::invoice(
                            &user.id,
                            sale.price,
                            &format!(
                                "hackmarket purchase buying {} at {}gp :{}:{} from <@{}>",
                                page.possession.name,
                                sale.price,
                                page.possession.id,
                                page.possession.kind.category() as u8,
                                page.possession.steader,
                            ),
                        )
                        .await?;
                    }

                    return Ok(ActionResponse::Json(Json(json!({
                        "response_action": "clear",
                    }))));
                }
                _ => {}
            };

            if let Some(Value::String(nickname)) = values
                .get("gotchi_nickname_block")
                .and_then(|i| i.get("gotchi_nickname_input"))
                .and_then(|s| s.get("value"))
            {
                // update the nickname in the DB
                let db = dyn_db();
                db.update_item(rusoto_dynamodb::UpdateItemInput {
                    table_name: hacksteader::TABLE_NAME.to_string(),
                    key: page.possession.key().into_item(),
                    update_expression: Some("SET nickname = :new_name".to_string()),
                    expression_attribute_values: Some(
                        [(
                            ":new_name".to_string(),
                            AttributeValue {
                                s: Some(nickname.clone()),
                                ..Default::default()
                            },
                        )]
                        .iter()
                        .cloned()
                        .collect(),
                    ),
                    ..Default::default()
                })
                .await
                .map_err(|e| format!("Couldn't change nickname in database: {}", e))?;

                let gotchi = page
                    .possession
                    .kind
                    .gotchi_mut()
                    .ok_or("can only nickname gotchi".to_string())?;

                // update the nickname on the Gotchi,
                gotchi.nickname = nickname.clone();
                let page_json = serde_json::to_string(&page).unwrap();

                // update the page in the background with the new gotchi data
                page.modal_update(trigger_id.to_string(), page_json, view.root_view_id)
                    .launch()
                    .await?;

                // update the home tab
                // TODO: make this not read from the database
                update_user_home_tab(user.id.clone()).await?;

                // this will close the "enter nickname" modal
                return Ok(ActionResponse::Ok(()));
            } else if let Some(price) = values
                .get("possession_sell_price_block")
                .and_then(|i| i.get("possession_sell_price_input"))
                .and_then(|s| s.get("value"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<u64>().ok())
            {
                banker::invoice(
                    &user.id,
                    price / 20_u64,
                    &format!(
                        "hackmarket fees for selling {} at {}gp :{}:{}",
                        page.possession.name,
                        price,
                        page.possession.id,
                        page.possession.kind.category() as u8
                    ),
                )
                .await?;

                return Ok(ActionResponse::Ok(()));
            } else if let Some(Value::String(new_owner)) = values
                .get("possession_give_receiver_block")
                .and_then(|i| i.get("possession_give_receiver_input"))
                .and_then(|s| s.get("selected_user"))
            {
                println!(
                    "giving {} from {} to {}",
                    view.private_metadata, user.id, new_owner
                );

                if user.id == *new_owner {
                    println!("self giving attempted");

                    return Ok(ActionResponse::Json(Json(json!({
                        "response_action": "errors",
                        "errors": {
                            "possession_give_receiver_input": "absolutely not okay",
                        }
                    }))));
                }

                // update the owner in the DB
                Hacksteader::transfer_possession(
                    &dyn_db(),
                    new_owner.clone(),
                    hacksteader::Acquisition::Trade,
                    &mut page.possession,
                )
                .await
                .map_err(|e| dbg!(e))?;

                // update the home tab
                // TODO: make this not read from the database
                update_user_home_tab(user.id.clone()).await?;

                // DM the new_owner about their new acquisition!
                dm_blocks(new_owner.clone(), {
                    // TODO: with_capacity optimization
                    let mut blocks = vec![
                        json!({
                            "type": "section",
                            "text": mrkdwn(format!(
                                "<@{}> has been so kind as to gift you a {:?}, {} _{}_!",
                                user.id,
                                page.possession.kind.category(),
                                emojify(&page.possession.name),
                                page.possession.nickname()
                            ))
                        }),
                        json!({ "type": "divider" }),
                    ];
                    page.interactivity = Interactivity::Read;
                    blocks.append(&mut page.blocks());
                    blocks.push(json!({ "type": "divider" }));
                    blocks.push(comment(format!(
                        "Manage all of your Hackagotchi at your <slack://app?team=T0266FRGM&id={}&tab=home|hackstead>",
                        *APP_ID,
                    )));
                    blocks
                }).await?;

                // close ALL THE MODALS!!!
                return Ok(ActionResponse::Json(Json(json!({
                    "response_action": "clear",
                }))));
            }
        } else if let Some((_view, None, _trigger_id, values, _user)) = view {
            println!("view state values: {:#?}", values);

            if let Some((tile_id, seed_id, seed_ah)) = dbg!(values.get("seed_plant_input"))
                .and_then(|i| dbg!(i.get("seed_plant_select")))
                .and_then(|s| dbg!(s.get("selected_option")))
                .and_then(|s| dbg!(s.get("value")))
                .and_then(|s| s.as_str())
                .and_then(|v| dbg!(serde_json::from_str(v).ok()))
            {
                println!("planting seed!");
                let db = dyn_db();
                futures::try_join!(
                    Hacksteader::delete(&db, hacksteader::Key::misc(seed_id)),
                    hacksteader::Tile::plant(&db, tile_id, seed_ah)
                )
                .map_err(|e| dbg!(format!("couldn't plant seed: {}", e)))?;

                return Ok(ActionResponse::Ok(()));
            }
        }
    }

    let mut i: Interaction =
        serde_json::from_str(&action_data.payload).map_err(|e| dbg!(format!("bad data: {}", e)))?;

    println!("{:#?}", i);

    Ok(ActionResponse::Json(Json(
        if let Some(action) = i.actions.pop() {
            match action
                .action_id
                .or(action.name)
                .ok_or("no action name".to_string())?
                .as_str()
            {
                "hackstead_confirm" => {
                    banker::invoice(&i.user.id, *HACKSTEAD_PRICE, "let's hackstead, fred!")
                        .await
                        .map_err(|e| format!("couldn't send Banker invoice DM: {}", e))?;

                    mrkdwn("Check your DMs from Banker for the hacksteading invoice!")
                }
                "possession_sell" => {
                    let page_json = i.view.ok_or("no view!".to_string())?.private_metadata;
                    //let page: PossessionPage = serde_json::from_str(&page_json).map_err(|e| dbg!(format!("couldn't parse {}: {}", page_json, e)))?;

                    Modal {
                        method: "push".to_string(),
                        trigger_id: i.trigger_id,
                        callback_id: "possession_sell_modal".to_string(),
                        title: "Sell Gotchi".to_string(),
                        private_metadata: page_json,
                        blocks: vec![
                            json!({
                                "type": "input",
                                "block_id": "possession_sell_price_block",
                                "label": plain_text("Price (gp)"),
                                "element": {
                                    "type": "plain_text_input",
                                    "action_id": "possession_sell_price_input",
                                    "placeholder": plain_text("Price Gotchi"),
                                    "initial_value": "50",
                                }
                            }),
                            json!({ "type": "divider" }),
                            comment("As a form of confirmation, you'll get an invoice to pay before your Gotchi goes up on the market. \
                                To fund Harvests and to encourage Hacksteaders to keep prices sensible, \
                                this invoice is 5% of the price of your sale \
                                rounded down to the nearest GP (meaning that sales below 20gp aren't taxed at all)."),
                        ],
                        submit: Some("Sell!".to_string()),
                        ..Default::default()
                    }
                    .launch()
                    .await?
                }
                "possession_give" => {
                    let page_json = i.view.ok_or("no view!".to_string())?.private_metadata;
                    let page: PossessionPage = serde_json::from_str(&page_json)
                        .map_err(|e| dbg!(format!("couldn't parse {}: {}", page_json, e)))?;

                    Modal {
                        method: "push".to_string(),
                        trigger_id: i.trigger_id,
                        callback_id: "possession_give_modal".to_string(),
                        title: "Give Gotchi".to_string(),
                        blocks: vec![json!({
                            "type": "input",
                            "block_id": "possession_give_receiver_block",
                            "label": plain_text("Give Gotchi"),
                            "element": {
                                "type": "users_select",
                                "action_id": "possession_give_receiver_input",
                                "placeholder": plain_text("Who Really Gets your Gotchi?"),
                                "initial_user": ({
                                    let s = &config::CONFIG.special_users;
                                    &s.get(page_json.len() % s.len()).unwrap_or(&*ID)
                                }),
                                "confirm": {
                                    "title": plain_text("You sure?"),
                                    "text": mrkdwn(format!(
                                        "Are you sure you want to give away {} _{}_? You might not get them back. :frowning:",
                                        emojify(&page.possession.name),
                                        page.possession.nickname()
                                    )),
                                    "confirm": plain_text("Give!"),
                                    "deny": plain_text("No!"),
                                    "style": "danger",
                                }
                            }
                        })],
                        private_metadata: page_json,
                        submit: Some("Trade Away!".to_string()),
                        ..Default::default()
                    }
                    .launch()
                    .await?
                }
                "seed_plant" => {
                    let (tile_id, seeds): (uuid::Uuid, Vec<Possessed<hacksteader::Seed>>) =
                        serde_json::from_str(&action.value).unwrap();

                    Modal {
                        method: "open".to_string(),
                        trigger_id: i.trigger_id,
                        callback_id: "seed_plant_modal".to_string(),
                        title: "Plant a Seed!".to_string(),
                        private_metadata: String::new(),
                        blocks: vec![json!({
                            "type": "input",
                            "label": plain_text("Seed Select"),
                            "block_id": "seed_plant_input",
                            "element": {
                                "type": "static_select",
                                "placeholder": plain_text("Which seed do ya wanna plant?"),
                                "action_id": "seed_plant_select",
                                // show them each seed they have that grows a given plant
                                "option_groups": config::CONFIG
                                    .plant_archetypes
                                    .iter()
                                    .enumerate()
                                    .map(|(pah, pa)| { //plant archetype handle, plant archetype
                                        json!({
                                            "label": plain_text(&pa.name),
                                            "options": seeds
                                                .iter()
                                                .filter(|s| s.inner.grows_into == pa.name)
                                                .map(|s| {
                                                    json!({
                                                        "text": plain_text(format!("{} {}", emojify(&s.name), s.name)),
                                                        "description": plain_text(format!(
                                                            "{} generations old", 
                                                            s.inner.pedigree.iter().map(|sg| sg.generations).sum::<u64>()
                                                        )),
                                                        // this is fucky-wucky because value can only be 75 chars
                                                        "value": serde_json::to_string(&(
                                                            &tile_id.to_simple().to_string(),
                                                            s.id.to_simple().to_string(),
                                                            pah
                                                        )).unwrap(),
                                                    })
                                                })
                                                .collect::<Vec<Value>>(),
                                        })
                                    })
                                    .collect::<Vec<Value>>(),
                            }
                        })],
                        submit: Some("Plant it!".to_string()),
                    }
                    .launch()
                    .await?
                }
                "gotchi_nickname" => {
                    Modal {
                        method: "push".to_string(),
                        trigger_id: i.trigger_id,
                        callback_id: "gotchi_nickname_modal".to_string(),
                        title: "Nickname Gotchi".to_string(),
                        private_metadata: i.view.ok_or("no view!".to_string())?.private_metadata,
                        blocks: vec![json!({
                            "type": "input",
                            "block_id": "gotchi_nickname_block",
                            "label": plain_text("Nickname Gotchi"),
                            "element": {
                                "type": "plain_text_input",
                                "action_id": "gotchi_nickname_input",
                                "placeholder": plain_text("Nickname Gotchi"),
                                "initial_value": action.value,
                                "min_length": 1,
                                "max_length": 25,
                            }
                        })],
                        submit: Some("Change it!".to_string()),
                        ..Default::default()
                    }
                    .launch()
                    .await?
                }
                "possession_market_page" => {
                    let market_json = action.value;
                    let possession: Possession = serde_json::from_str(&market_json).unwrap();

                    let page = PossessionPage {
                        credentials: dbg!(if i.user.id == possession.steader {
                            Credentials::Owner
                        } else if hacksteader::exists(&dyn_db(), i.user.id.clone()).await {
                            Credentials::Hacksteader
                        } else {
                            Credentials::None
                        }),
                        possession,
                        interactivity: Interactivity::Buy,
                    };

                    let page_json = serde_json::to_string(&page).unwrap();
                    page.modal(i.trigger_id, page_json).launch().await?
                }
                "possession_page" => {
                    let page_json = action.value;
                    let page: PossessionPage = serde_json::from_str(&page_json).unwrap();

                    page.modal(i.trigger_id, page_json).launch().await?
                }
                _ => mrkdwn("huh?"),
            }
        } else {
            mrkdwn("no actions?")
        },
    )))
}

#[derive(serde::Deserialize, Debug)]
struct ChallengeEvent {
    challenge: String,
}
#[post("/event", format = "application/json", data = "<event_data>", rank = 2)]
async fn challenge(event_data: Json<ChallengeEvent>) -> String {
    println!("challenge");
    event_data.challenge.clone()
}

#[derive(serde::Deserialize, Debug, Clone)]
struct Event<'a> {
    #[serde(borrow, rename = "event")]
    reply: Message<'a>,
}
#[derive(serde::Deserialize, Debug, Clone)]
pub struct Message<'a> {
    #[serde(rename = "user")]
    user_id: String,
    channel: String,
    #[serde(default)]
    text: String,
    #[serde(rename = "type")]
    kind: &'a str,
    tab: Option<&'a str>,
}

#[post("/event", format = "application/json", data = "<e>", rank = 1)]
async fn event(active_users: State<'_, Sender<String>>, e: Json<Event<'_>>) -> Result<(), String> {
    use config::CONFIG;

    let Event { reply: r } = (*e).clone();
    println!("{:#?}", r);

    lazy_static::lazy_static! {
        static ref SPAWN_POSSESSION_REGEX: regex::Regex = regex::Regex::new(
            "<@([A-z|0-9]+)> spawn (<@([A-z|0-9]+)> )?(.+)"
        ).unwrap();
        static ref DUMP_GP_REGEX: regex::Regex = regex::Regex::new(
            "<@([A-z|0-9]+)> dump <@([A-z|0-9]+)> ([0-9]+)"
        ).unwrap();
        static ref LANDFILL_REGEX: regex::Regex = regex::Regex::new(
            "<@([A-z|0-9]+)> landfill"
        ).unwrap();
        static ref MARKET_FEES_INVOICE_REGEX: regex::Regex = regex::Regex::new(
            "hackmarket fees for selling (.+) at ([0-9]+)gp :(.+):([0-9])",
        ).unwrap();
        static ref MARKET_PURCHASE_REGEX: regex::Regex = regex::Regex::new(
            "hackmarket purchase buying (.+) at ([0-9]+)gp :(.+):([0-9]) from <@([A-z|0-9]+)>",
        ).unwrap();
        static ref BALANCE_REPORT_REGEX: regex::Regex = regex::Regex::new(
            "You have ([0-9]+)gp in your account, sirrah."
        ).unwrap();
    }

    // TODO: clean these three mofos up
    if let Message {
        kind: "app_home_opened",
        tab: Some("home"),
        ref user_id,
        ..
    } = r
    {
        println!("Rendering app_home!");
        active_users
            .send(user_id.clone())
            .expect("couldn't send active user");
        update_user_home_tab(user_id.clone()).await?;
    } else if let Some(paid_invoice) =
        banker::parse_paid_invoice(&r).filter(|pi| pi.invoicer == *ID)
    {
        println!("invoice {:#?} just paid", paid_invoice);

        struct Sale {
            name: String,
            price: u64,
            id: uuid::Uuid,
            category: hacksteader::Category,
            from: Option<String>,
        }

        fn captures_to_sale(captures: &regex::Captures) -> Option<Sale> {
            Some(Sale {
                name: captures.get(1)?.as_str().to_string(),
                price: captures.get(2)?.as_str().parse().ok()?,
                id: uuid::Uuid::parse_str(captures.get(3)?.as_str()).ok()?,
                category: captures
                    .get(4)?
                    .as_str()
                    .parse::<u8>()
                    .ok()?
                    .try_into()
                    .ok()?,
                from: captures.get(5).map(|x| x.as_str().to_string()),
            })
        }

        if paid_invoice.reason == "let's hackstead, fred!" {
            Hacksteader::new_in_db(&dyn_db(), paid_invoice.invoicee.clone())
                .await
                .map_err(|_| "Couldn't put you in the hacksteader database!")?;

            dm_blocks(paid_invoice.invoicee.clone(), vec![
                 json!({
                     "type": "section",
                     "text": mrkdwn(
                         "Congratulations, new Hacksteader! \
                         Welcome to the community!\n\n\
                         :house: Manage your hackstead in the home tab above\n\
                         :harder-flex: Show anyone a snapshot of your hackstead with /hackstead\n\
                         :sleuth_or_spy: Look up anyone else's hackstead with /hackstead @<their name>\n\
                         :money_with_wings: Shop from the user-run hacksteaders' market with /hackmarket\n\n\
                         You might want to start by buying some seeds there and planting them at your hackstead!"
                     ),
                 }),
                 comment("LET'S HACKSTEAD, FRED!")
            ]).await?;
        } else if let Some(captures) = MARKET_FEES_INVOICE_REGEX.captures(&paid_invoice.reason) {
            println!("MARKET_FEES_INVOICE_REGEX: {:#?}", captures);

            if let Some(Sale {
                name,
                price,
                id,
                category,
                ..
            }) = captures_to_sale(&captures)
            {
                market::place_on_market(&dyn_db(), category, id, price, name).await?;
            }
        } else if let Some(captures) = MARKET_PURCHASE_REGEX.captures(&paid_invoice.reason) {
            use futures::future::TryFutureExt;
            println!("MARKET_PURCHASE_REGEX: {:#?}", captures);

            if let Some(Sale {
                id,
                category,
                name,
                price,
                from: Some(seller),
            }) = captures_to_sale(&captures)
            {
                let paid_for = format!("sale of your {}", name);
                let db = dyn_db();
                futures::try_join!(
                    db.update_item(rusoto_dynamodb::UpdateItemInput {
                        key: [
                            ("cat".to_string(), category.into_av()),
                            (
                                "id".to_string(),
                                AttributeValue {
                                    s: Some(id.to_string()),
                                    ..Default::default()
                                },
                            ),
                        ]
                        .iter()
                        .cloned()
                        .collect(),
                        expression_attribute_values: Some([
                            (":new_owner".to_string(), AttributeValue {
                                s: Some(paid_invoice.invoicee.clone()),
                                ..Default::default()
                            }),
                            (":ownership_entry".to_string(), AttributeValue {
                                l: Some(vec![hacksteader::Owner {
                                    id: paid_invoice.invoicee.clone(),
                                    acquisition: hacksteader::Acquisition::Purchase {
                                        price: paid_invoice.amount,
                                    }
                                }.into()]),
                                ..Default::default()
                            })
                        ].iter().cloned().collect()),
                        update_expression: Some(concat!(
                            "REMOVE price, market_name ",
                            "SET steader = :new_owner, ownership_log = list_append(ownership_log, :ownership_entry)"
                        ).to_string()),
                        table_name: hacksteader::TABLE_NAME.to_string(),
                        ..Default::default()
                    }).map_err(|e| format!("database err: {}", e)),
                    banker::pay(seller.clone(), price, paid_for),
                    dm_blocks(seller.clone(), vec![
                        json!({
                            "type": "section",
                            "text": mrkdwn(format!(
                                "The sale of your *{}* has gone through! \
                                <@{}> made the purchase on hackmarket, earning you *{} GP*!",
                                name, paid_invoice.invoicee, price
                            )),
                            "accessory": {
                                "type": "image",
                                "image_url": format!("http://{}/gotchi/img/gotchi/{}.png", *URL, filify(name)),
                                "alt_text": "Hackpheus sitting on bags of money!",
                            }
                        }),
                        comment("BRUH UR LIKE ROLLING IN CASH"),
                    ])
                )
                .map_err(|e| dbg!(format!("Couldn't complete sale of {}: {}", id, e)))?;
            }
        }

        banker::balance().await?;
    }
    if let Some(balance) = dbg!(BALANCE_REPORT_REGEX.captures(&r.text))
        .filter(|_| r.channel == *banker::CHAT_ID)
        .and_then(|x| x.get(1))
        .and_then(|x| x.as_str().parse::<u64>().ok())
    {
        use futures::future::TryFutureExt;
        use futures::stream::{self, StreamExt, TryStreamExt};
        println!("I got {} problems and GP ain't one", balance);

        let query = dyn_db()
            .query(rusoto_dynamodb::QueryInput {
                table_name: hacksteader::TABLE_NAME.to_string(),
                key_condition_expression: Some("cat = :gotchi_cat".to_string()),
                expression_attribute_values: Some({
                    [(
                        ":gotchi_cat".to_string(),
                        hacksteader::Category::Gotchi.into_av(),
                    )]
                    .iter()
                    .cloned()
                    .collect()
                }),
                ..Default::default()
            })
            .await;

        let gotchis = query
            .map_err(|e| format!("couldn't query all gotchis: {}", e))?
            .items
            .ok_or("no gotchis found!")?
            .iter()
            .filter_map(|i| match Possession::from_item(i) {
                Ok(p) => Some(Possessed::<Gotchi>::from_possession(p).unwrap()),
                Err(e) => {
                    println!("error parsing gotchi: {}", e);
                    None
                }
            })
            .collect::<Vec<Possessed<Gotchi>>>();

        let total_happiness: u64 = gotchis.iter().map(|x| x.inner.base_happiness).sum();
        let mut funds_awarded = 0;

        for _ in 0..balance / total_happiness {
            stream::iter(gotchis.clone()).map(|x| Ok(x)).try_for_each_concurrent(None, |gotchi| {
                let dm = vec![
                    json!({
                        "type": "section",
                        "text": mrkdwn(format!(
                            "_It's free GP time, ladies and gentlegotchis!_\n\n\
                            It seems your lovely Gotchi *{}* has collected *{} GP* for you!",
                            gotchi.inner.nickname,
                            gotchi.inner.base_happiness
                        )),
                        "accessory": {
                            "type": "image",
                            "image_url": format!("http://{}/gotchi/img/gotchi/{}.png", *URL, filify(&gotchi.name)),
                            "alt_text": "Hackpheus holding a Gift!",
                        }
                    }),
                    comment("IN HACK WE STEAD")
                ];
                let payment_note = format!(
                    "{} collected {} GP for you",
                    gotchi.inner.nickname,
                    gotchi.inner.base_happiness,
                );
                let steader_in_harvest_log: bool = gotchi.inner.harvest_log.last().filter(|x| x.id == gotchi.steader).is_some();
                let db_update = rusoto_dynamodb::UpdateItemInput {
                    table_name: hacksteader::TABLE_NAME.to_string(),
                    key: gotchi.clone().into_possession().key().into_item(),
                    update_expression: Some(if steader_in_harvest_log {
                        format!("ADD harvest_log[{}].harvested :harv", gotchi.inner.harvest_log.len() - 1)
                    } else {
                        "SET harvest_log = list_append(harvest_log, :harv)".to_string()
                    }),
                    expression_attribute_values: Some(
                        [(
                            ":harv".to_string(),
                            if steader_in_harvest_log {
                                AttributeValue {
                                    n: Some(gotchi.inner.base_happiness.to_string()),
                                    ..Default::default()
                                }
                            } else {
                                AttributeValue {
                                    l: Some(vec![hacksteader::GotchiHarvestOwner {
                                        id: gotchi.steader.clone(),
                                        harvested: gotchi.inner.base_happiness,
                                    }.into()]),
                                    ..Default::default()
                                }
                            }
                        )]
                        .iter()
                        .cloned()
                        .collect(),
                    ),
                    ..Default::default()
                };
                let db = dyn_db();

                async move {
                    futures::try_join!(
                        dm_blocks(gotchi.steader.clone(), dm),
                        banker::pay(gotchi.steader.clone(), gotchi.inner.base_happiness, payment_note),
                        db.update_item(db_update).map_err(|e| format!("Couldn't update owner log: {}", e))
                    )?;
                    Ok(())
                }
            })
            .await
            .map_err(|e: String| format!("harvest msg send err: {}", e))?;

            funds_awarded += total_happiness;
        }

        futures::try_join!(
            banker::message(format!("{} GP earned this harvest!", funds_awarded)),
            banker::message(format!("total happiness: {}", total_happiness)),
        )?;
    } else if CONFIG.special_users.contains(&r.user_id) {
        if let Some((receiver, archetype_handle)) = dbg!(SPAWN_POSSESSION_REGEX.captures(&r.text))
            .and_then(|c| {
                let _spawner = c.get(1).filter(|x| x.as_str() == &*ID)?;
                let receiver = c.get(3).map(|x| x.as_str()).unwrap_or(&r.user_id);
                let possession_name = c.get(4)?.as_str();
                let archetype_handle = CONFIG
                    .possession_archetypes
                    .iter()
                    .position(|x| x.name == possession_name)?;
                Some((receiver.to_string(), archetype_handle))
            })
        {
            Hacksteader::give_possession(
                &dyn_db(),
                receiver.clone(),
                &Possession::new(
                    archetype_handle,
                    hacksteader::Owner {
                        id: receiver,
                        acquisition: hacksteader::Acquisition::spawned(),
                    },
                ),
            )
            .await
            .map_err(|_| "hacksteader database problem")?;
        }
        if let Some((dump_to, dump_amount)) = dbg!(DUMP_GP_REGEX.captures(&r.text)).and_then(|c| {
            let _dumper = c.get(1).filter(|x| x.as_str() == &*ID)?;
            Some((
                c.get(2)?.as_str().to_string(),
                c.get(3)?.as_str().parse::<u64>().ok()?,
            ))
        }) {
            println!("dumping {} to {}", dump_amount, dump_to);
            dbg!(banker::pay(dump_to, dump_amount, "GP dump".to_string()).await?);
        }
        if dbg!(LANDFILL_REGEX.captures(&r.text))
            .and_then(|c| c.get(1))
            .filter(|x| x.as_str() == &*ID)
            .is_some()
        {
            println!("landfill time!");

            match hacksteader::landfill(&dyn_db(), &*active_users).await {
                Ok(()) => {}
                Err(e) => println!("landfill error: {}", e),
            }
        }
    }

    Ok(())
}

#[rocket::get("/steadercount")]
async fn steadercount() -> Result<String, String> {
    hacksteader::Profile::fetch_all(&dyn_db())
        .await
        .map(|profiles| profiles.len().to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    use rocket_contrib::serve::StaticFiles;

    let (tx, rx) = crossbeam_channel::unbounded();

    tokio::task::spawn({
        use std::collections::HashMap;
        use std::time::{Duration, SystemTime};
        use tokio::time::interval;

        const FARM_CYCLE_MILLIS: u64 = 15 * 1000;
        const TIME_FILE: &'static str = concat!(env!("CARGO_MANIFEST_DIR"), "/last_farming.time");

        let mut interval = interval(Duration::from_millis(FARM_CYCLE_MILLIS));

        let mut active_users: HashMap<String, bool> = HashMap::new();

        let mut last_farm = {
            let raw = std::fs::read_to_string(TIME_FILE)
                .unwrap_or_else(|e| panic!("couldn't read {}: {}", TIME_FILE, e));
            humantime::parse_rfc3339(raw.trim()).unwrap_or_else(|e| {
                panic!("couldn't parse {} from {}: {}", raw.trim(), TIME_FILE, e)
            })
        };

        async move {
            use futures::future::TryFutureExt;
            use futures::stream::{self, StreamExt, TryStreamExt};
            use hacksteader::{Plant, Profile, Tile};
            use std::collections::HashMap;

            loop {
                for (_, fresh) in active_users.iter_mut() {
                    *fresh = false;
                }

                while let Ok(user) = rx.try_recv() {
                    println!("active: {}", user);
                    active_users.insert(user, true);
                }

                interval.tick().await;

                if active_users.is_empty() {
                    continue;
                }

                let db = dyn_db();

                let elapsed = SystemTime::now()
                    .duration_since(last_farm)
                    .unwrap_or_default()
                    .as_millis()
                    / (FARM_CYCLE_MILLIS as u128);

                let hacksteaders: Vec<Hacksteader> = match stream::iter(active_users.clone())
                    .map(|(id, _)| Hacksteader::from_db(&db, id))
                    .buffer_unordered(50)
                    .try_collect::<Vec<_>>()
                    .await
                {
                    Ok(i) => i,
                    Err(e) => {
                        println!("error reading hacksteader from db: {}", e);
                        continue;
                    }
                };

                // we'll be frequently looking up profiles by who owns them to award xp.
                let mut profiles: HashMap<String, Profile> = hacksteaders
                    .iter()
                    .map(|hs| (hs.user_id.clone(), hs.profile.clone()))
                    .collect();

                // we can only farm on tiles with plants,
                let mut tiles: Vec<(Plant, Tile)> = hacksteaders
                    .into_iter()
                    .flat_map(|hs| {
                        hs.land
                            .into_iter()
                            .filter_map(|mut t| Some((t.plant.take()?, t)))
                    })
                    .collect();

                let mut dms: Vec<(String, [Value; 2])> = Vec::new();

                for ((_, profile), (user, fresh)) in
                    profiles.iter_mut().zip(active_users.clone().into_iter())
                {
                    let now = std::time::SystemTime::now();
                    if fresh {
                        profile.last_active = now;
                    } else {
                        const ACTIVE_DURATION: u64 = 60 * 5;
                        if now
                            .duration_since(profile.last_active)
                            .ok()
                            .filter(|r| dbg!(r.as_secs()) >= ACTIVE_DURATION)
                            .is_some()
                        {
                            active_users.remove(&user);
                        }
                    }
                }

                println!("triggering {} farming cycles", elapsed);
                for _ in 0..elapsed {
                    for (plant, tile) in tiles.iter_mut() {
                        match profiles.get_mut(&tile.steader) {
                            Some(profile) => {
                                let sum = profile.advancements.sum(profile.xp);
                                if let Some(advancement) = profile.increment_xp() {
                                    dms.push((tile.steader.clone(), [
                                        json!({
                                            "type": "section",
                                            "text": mrkdwn(format!(
                                                concat!(
                                                    ":tada: Your Hackstead is now a *{}*!\n\n",
                                                    "*{}* Achieved:\n_{}_\n\n",
                                                    ":stonks: Total XP: *{}xp*\n",
                                                    ":mountain: Land Available: *{} pieces* _(+{} pieces)_"
                                                ),
                                                advancement.achiever_title,
                                                advancement.title,
                                                advancement.description,
                                                advancement.xp,
                                                sum.land,
                                                match advancement.kind {
                                                    config::HacksteadAdvancementKind::Land { pieces } => pieces,
                                                }
                                            )),
                                            "accessory": {
                                                "type": "image",
                                                "image_url": format!("http://{}/gotchi/img/gotchi/hackpheus.png", *URL),
                                                "alt_text": "happy shiny better hackstead",
                                            }
                                        }),
                                        comment("SUPER EXCITING LEVELING UP NOISES"),
                                    ]));
                                }
                                if let Some(advancement) = plant.increment_xp() {
                                    dms.push((tile.steader.clone(), [
                                        json!({
                                            "type": "section",
                                            "text": mrkdwn(format!(
                                                concat!(
                                                    ":tada: Your {} is now a *{}*!\n\n",
                                                    "*{}* Achieved:\n _{}_\n\n",
                                                    ":stonks: Total XP: *{}xp*",
                                                ),
                                                plant.name,
                                                advancement.achiever_title,
                                                advancement.title,
                                                advancement.description,
                                                advancement.xp
                                            )),
                                            "accessory": {
                                                "type": "image",
                                                "image_url": format!("http://{}/gotchi/img/plant/{}.png", *URL, filify(&plant.name)),
                                                "alt_text": "happy shiny better plant",
                                            }
                                        }),
                                        comment("EXCITING LEVELING UP NOISES"),
                                    ]))
                                }
                            }
                            None => {
                                println!(
                                    "couldn't get tile[{}]'s steader[{}]'s profile",
                                    tile.id, tile.steader
                                );
                            }
                        }
                    }
                }

                let _ = futures::try_join!(
                    stream::iter(profiles.clone())
                        .map(|x| Ok(x))
                        .try_for_each_concurrent(None, |(who, _)| { update_user_home_tab(who) }),
                    stream::iter(dms)
                        .map(|x| Ok(x))
                        .try_for_each_concurrent(None, |(who, blocks)| {
                            dm_blocks(who.clone(), blocks.to_vec())
                        }),
                    db.batch_write_item(rusoto_dynamodb::BatchWriteItemInput {
                        request_items: [(
                            hacksteader::TABLE_NAME.to_string(),
                            tiles
                                .into_iter()
                                .map(|(plant, mut tile)| rusoto_dynamodb::WriteRequest {
                                    put_request: Some(rusoto_dynamodb::PutRequest {
                                        item: {
                                            tile.plant = Some(plant);

                                            tile.into_av().m.expect("tile attribute should be map")
                                        }
                                    }),
                                    ..Default::default()
                                })
                                .chain(profiles.iter().map(|(_, p)| {
                                    rusoto_dynamodb::WriteRequest {
                                        put_request: Some(rusoto_dynamodb::PutRequest {
                                            item: p.item(),
                                        }),
                                        ..Default::default()
                                    }
                                }))
                                .collect(),
                        )]
                        .iter()
                        .cloned()
                        .collect(),
                        ..Default::default()
                    })
                    .map_err(|e| format!("couldn't write new land into db: {}", e))
                )
                .map_err(|e| println!("farm cycle async err: {}", e));

                if elapsed > 0 {
                    last_farm += Duration::from_millis(
                        (FARM_CYCLE_MILLIS as u128 * elapsed)
                            .try_into()
                            .unwrap_or_else(|e| {
                                println!(
                                    "too many farm cycle millis * elapsed[{}]: {}",
                                    elapsed, e
                                );
                                0
                            }),
                    );
                    std::fs::write(
                        TIME_FILE,
                        humantime::format_rfc3339_millis(last_farm).to_string(),
                    )
                    .unwrap_or_else(|e| {
                        println!(
                            "couldn't write latest farm time {:?} to {:?}: {}",
                            last_farm, TIME_FILE, e
                        );
                    });
                }
            }
        }
    });

    dotenv::dotenv().ok();

    rocket::ignite()
        .manage(tx)
        .mount(
            "/gotchi",
            routes![
                hackstead,
                hackmarket,
                action_endpoint,
                challenge,
                event,
                steadercount
            ],
        )
        .mount(
            "/gotchi/img",
            StaticFiles::from(concat!(env!("CARGO_MANIFEST_DIR"), "/img")),
        )
        .serve()
        .await
        .expect("launch fail");

    Ok(())
}
