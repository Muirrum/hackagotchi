use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub enum ConfigError {
    UnknownArchetypeName(String),
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ConfigError::*;
        match self {
            UnknownArchetypeName(name) => write!(f, "no archetype by the name of {:?}", name),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub special_users: Vec<String>,
    pub profile_archetype: ProfileArchetype,
    pub plant_archetypes: Vec<PlantArchetype>,
    pub possession_archetypes: Vec<Archetype>,
}
impl Config {
    #[allow(dead_code)]
    pub fn find_plant<S: AsRef<str>>(&self, name: &S) -> Result<&PlantArchetype, ConfigError> {
        self.plant_archetypes
            .iter()
            .find(|x| name.as_ref() == x.name)
            .ok_or(ConfigError::UnknownArchetypeName(name.as_ref().to_string()))
    }
    pub fn find_plant_handle<S: AsRef<str>>(
        &self,
        name: &S,
    ) -> Result<ArchetypeHandle, ConfigError> {
        self.plant_archetypes
            .iter()
            .position(|x| name.as_ref() == x.name)
            .ok_or(ConfigError::UnknownArchetypeName(name.as_ref().to_string()))
    }
    #[allow(dead_code)]
    pub fn find_possession<S: AsRef<str>>(&self, name: &S) -> Result<&Archetype, ConfigError> {
        self.possession_archetypes
            .iter()
            .find(|x| name.as_ref() == x.name)
            .ok_or(ConfigError::UnknownArchetypeName(name.as_ref().to_string()))
    }
    pub fn find_possession_handle<S: AsRef<str>>(
        &self,
        name: &S,
    ) -> Result<ArchetypeHandle, ConfigError> {
        self.possession_archetypes
            .iter()
            .position(|x| name.as_ref() == x.name)
            .ok_or(ConfigError::UnknownArchetypeName(name.as_ref().to_string()))
    }
}

// I should _really_ use a different version of this for PlantArchetypes and PossessionArchetypes ...
pub type ArchetypeHandle = usize;

lazy_static::lazy_static! {
    pub static ref CONFIG: Config = {
        pub fn f<T: DeserializeOwned>(p: &'static str) -> T {
            serde_json::from_str(
                &std::fs::read_to_string(format!(
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/config/{}.json",
                    ),
                    p
                ))
                .unwrap_or_else(|e| panic!("opening {}: {}", p, e))
            )
            .unwrap_or_else(|e| panic!("parsing {}: {}", p, e))
        }

        Config {
            special_users: f("special_users"),
            profile_archetype: ProfileArchetype {
                advancements: f("hackstead_advancements"),
            },
            plant_archetypes: f("plant_archetypes"),
            possession_archetypes: f("possession_archetypes"),
        }
    };
}

#[derive(Deserialize, Debug, Clone)]
pub struct ProfileArchetype {
    pub advancements: AdvancementSet<HacksteadAdvancementSum>,
}

pub type HacksteadAdvancement = Advancement<HacksteadAdvancementSum>;
pub type HacksteadAdvancementSet = AdvancementSet<HacksteadAdvancementSum>;
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub enum HacksteadAdvancementKind {
    Land { pieces: u32 },
}
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct HacksteadAdvancementSum {
    pub land: u32,
    pub xp: u64,
}
impl AdvancementSum for HacksteadAdvancementSum {
    type Kind = HacksteadAdvancementKind;

    fn new(unlocked: &[&Advancement<Self>]) -> Self {
        Self {
            xp: unlocked.iter().fold(0, |a, c| a + c.xp),
            land: unlocked
                .iter()
                .map(|k| match k.kind {
                    HacksteadAdvancementKind::Land { pieces } => pieces,
                })
                .sum(),
        }
    }

    fn filter_base(_a: &Advancement<Self>) -> bool {
        true
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct GotchiArchetype {
    pub base_happiness: u64,
    #[serde(default)]
    pub plant_effects: Option<(String, PlantAdvancement)>,
}
#[derive(Deserialize, Debug, Clone)]
pub struct SeedArchetype {
    pub grows_into: String,
}
#[derive(Deserialize, Debug, Clone)]
pub enum ApplicationEffect {
    TimeIncrease {
        extra_cycles: u64,
        duration_cycles: u64,
    },
}
#[derive(Deserialize, Debug, Clone)]
pub struct LandUnlock {
    pub requires_xp: bool,
}
#[derive(Deserialize, Debug, Clone)]
pub struct KeepsakeArchetype {
    pub item_application_effect: Option<ApplicationEffect>,
    pub unlocks_land: Option<LandUnlock>,
    pub plant_effects: Option<(String, PlantAdvancement)>,
}

#[derive(Deserialize, Debug, Clone)]
pub enum ArchetypeKind {
    Gotchi(GotchiArchetype),
    Seed(SeedArchetype),
    Keepsake(KeepsakeArchetype),
}
impl ArchetypeKind {
    pub fn category(&self) -> crate::Category {
        use crate::Category;
        match self {
            ArchetypeKind::Gotchi(_) => Category::Gotchi,
            _ => Category::Misc,
        }
    }
    pub fn keepsake(&self) -> Option<&KeepsakeArchetype> {
        match self {
            ArchetypeKind::Keepsake(k) => Some(k),
            _ => None,
        }
    }
}
#[derive(Deserialize, Debug, Clone)]
pub struct Archetype {
    pub name: String,
    pub description: String,
    pub kind: ArchetypeKind,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PlantArchetype {
    pub name: String,
    pub base_yield_duration: f32,
    pub advancements: AdvancementSet<PlantAdvancementSum>,
}
impl Eq for PlantArchetype {}
impl PartialEq for PlantArchetype {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Hash for PlantArchetype {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}
pub type PlantAdvancement = Advancement<PlantAdvancementSum>;
pub type PlantAdvancementSet = AdvancementSet<PlantAdvancementSum>;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum RecipeMakes<Handle: Clone> {
    Just(usize, Handle),
    OneOf(Vec<(f32, Handle)>),
    AllOf(Vec<(usize, Handle)>),
}
impl<Handle: Clone> RecipeMakes<Handle> {
    /// Returns one possible output, randomly (but properly weighted)
    /// if more than one is possible.
    pub fn any(&self) -> Handle {
        use rand::Rng;
        use RecipeMakes::*;

        match self {
            Just(_, h) => h.clone(),
            OneOf(these) => {
                let mut x: f32 = rand::thread_rng().gen_range(0.0, 1.0);
                these
                    .iter()
                    .find_map(|(chance, h)| {
                        x -= chance;
                        if x < 0.0 {
                            Some(h)
                        } else {
                            None
                        }
                    })
                    .unwrap()
                    .clone()
            }
            AllOf(these) => {
                let total = these.iter().map(|(count, _)| *count).sum::<usize>() as f32;
                OneOf(
                    these
                        .iter()
                        .map(|(count, h)| (*count as f32 / total, h.clone()))
                        .collect(),
                )
                .any()
            }
        }
    }
}
impl fmt::Display for RecipeMakes<&'static Archetype> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::frontend::emojify;
        use RecipeMakes::*;

        match self {
            Just(1, x) => write!(f, "a {} _{}_", emojify(&x.name), x.name),
            Just(n, x) => write!(f, "*{}* {} _{}_", n, emojify(&x.name), x.name),
            OneOf(these) => write!(
                f,
                "one of these:\n{}",
                these
                    .iter()
                    .map(|(chance, what)| {
                        format!(
                            "a {} _{}_ (*{:.2}%* chance)",
                            emojify(&what.name),
                            what.name,
                            chance
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            AllOf(these) => write!(
                f,
                "all of the following:\n{}",
                these
                    .iter()
                    .map(|(count, what)| {
                        format!("*{}* {} _{}_", emojify(&what.name), what.name, count)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        }
    }
}

impl RecipeMakes<ArchetypeHandle> {
    pub fn lookup_handles(self) -> Option<RecipeMakes<&'static Archetype>> {
        use RecipeMakes::*;

        fn lookup(ah: ArchetypeHandle) -> Option<&'static Archetype> {
            CONFIG.possession_archetypes.get(ah)
        }

        Some(match self {
            Just(n, ah) => Just(n, lookup(ah)?),
            OneOf(l) => OneOf(
                l.into_iter()
                    .map(|(c, ah)| Some((c, lookup(ah)?)))
                    .collect::<Option<_>>()?,
            ),
            AllOf(l) => AllOf(
                l.into_iter()
                    .map(|(c, ah)| Some((c, lookup(ah)?)))
                    .collect::<Option<_>>()?,
            ),
        })
    }
}

impl RecipeMakes<String> {
    pub fn find_handles(self) -> Result<RecipeMakes<ArchetypeHandle>, ConfigError> {
        use RecipeMakes::*;

        fn find(name: String) -> Result<ArchetypeHandle, ConfigError> {
            CONFIG.find_possession_handle(&name)
        }

        Ok(match self {
            Just(n, ah) => Just(n, find(ah)?),
            OneOf(l) => OneOf(
                l.into_iter()
                    .map(|(c, ah)| Ok((c, find(ah)?)))
                    .collect::<Result<_, _>>()?,
            ),
            AllOf(l) => AllOf(
                l.into_iter()
                    .map(|(c, ah)| Ok((c, find(ah)?)))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

/// Recipe is generic over the way Archetypes are referred to
/// to make it easy to use Strings in the configs and ArchetypeHandles
/// at runtime
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Recipe<Handle: Clone> {
    pub needs: Vec<(usize, Handle)>,
    pub makes: RecipeMakes<Handle>,
    #[serde(default)]
    pub destroys_plant: bool,
    pub time: f32,
}
impl Recipe<ArchetypeHandle> {
    pub fn satisfies(&self, inv: &[crate::Possession]) -> bool {
        self.needs.iter().copied().all(|(count, ah)| {
            let has = inv.iter().filter(|x| x.archetype_handle == ah).count();
            count <= has
        })
    }
    pub fn lookup_handles(self) -> Option<Recipe<&'static Archetype>> {
        Some(Recipe {
            makes: self.makes.lookup_handles()?,
            needs: self
                .needs
                .into_iter()
                .map(|(n, x)| Some((n, CONFIG.possession_archetypes.get(x)?)))
                .collect::<Option<Vec<(_, &Archetype)>>>()?,
            time: self.time,
            destroys_plant: self.destroys_plant,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct SpawnRate(pub f32, pub (f32, f32));
impl SpawnRate {
    pub fn gen_count<R: rand::Rng>(self, rng: &mut R) -> usize {
        let Self(guard, (lo, hi)) = self;
        if rng.gen_range(0.0, 1.0) < guard {
            let chance = rng.gen_range(lo, hi);
            let base = chance.floor();
            let extra = if rng.gen_range(0.0, 1.0) < chance - base {
                1
            } else {
                0
            };
            base as usize + extra
        } else {
            0
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub enum PlantAdvancementKind {
    Xp(f32),
    YieldSpeed(f32),
    YieldSize(f32),
    Neighbor(Box<PlantAdvancementKind>),
    Yield(Vec<(SpawnRate, String)>),
    Craft(Vec<Recipe<String>>),
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(bound(deserialize = ""))]
pub struct PlantAdvancementSum {
    pub xp: u64,
    pub xp_multiplier: f32,
    pub yield_speed_multiplier: f32,
    pub yield_size_multiplier: f32,
    pub yields: Vec<(SpawnRate, ArchetypeHandle)>,
    pub recipes: Vec<Recipe<ArchetypeHandle>>,
}
impl AdvancementSum for PlantAdvancementSum {
    type Kind = PlantAdvancementKind;

    fn new(unlocked: &[&Advancement<Self>]) -> Self {
        use PlantAdvancementKind::*;

        let mut xp = 0;
        let mut xp_multiplier = 1.0;
        let mut yield_speed_multiplier = 1.0;
        let mut yield_size_multiplier = 1.0;
        let mut yields = vec![];
        let mut recipes = vec![];

        for k in unlocked.iter() {
            xp += k.xp;

            // apply neighbor upgrades as if they weren't neighbor upgrades :D
            let kind = match &k.kind {
                Neighbor(n) => &**n,
                other => other,
            };

            match kind {
                Xp(multiplier) => xp_multiplier *= multiplier,
                YieldSpeed(multiplier) => yield_speed_multiplier *= multiplier,
                Neighbor(..) => {}
                YieldSize(multiplier) => yield_size_multiplier *= multiplier,
                Yield(resources) => yields.append(
                    &mut resources
                        .iter()
                        .map(|(c, s)| Ok((*c, CONFIG.find_possession_handle(s)?)))
                        .collect::<Result<Vec<_>, ConfigError>>()
                        .expect("couldn't find archetype for advancement yield"),
                ),
                Craft(new_recipes) => recipes.append(
                    &mut new_recipes
                        .iter()
                        .map(|r| {
                            Ok(Recipe {
                                makes: r.makes.clone().find_handles()?,
                                needs: r
                                    .needs
                                    .iter()
                                    .map(|(c, s)| Ok((*c, CONFIG.find_possession_handle(s)?)))
                                    .collect::<Result<Vec<_>, ConfigError>>()?,

                                time: r.time,
                                destroys_plant: r.destroys_plant,
                            })
                        })
                        .collect::<Result<Vec<_>, ConfigError>>()
                        .expect("couldn't find archetype for crafting advancement"),
                ),
            }
        }

        yields = yields
            .into_iter()
            .map(|(SpawnRate(guard, (lo, hi)), name)| {
                (
                    SpawnRate(
                        (guard * yield_size_multiplier).min(1.0),
                        (lo * yield_size_multiplier, hi * yield_size_multiplier),
                    ),
                    name,
                )
            })
            .collect();

        Self {
            xp,
            xp_multiplier,
            yield_speed_multiplier,
            yield_size_multiplier,
            yields,
            recipes,
        }
    }

    // ignore your neighbor bonuses you give out
    fn filter_base(a: &Advancement<Self>) -> bool {
        match &a.kind {
            PlantAdvancementKind::Neighbor(..) => false,
            _ => true,
        }
    }
}

pub trait AdvancementSum: DeserializeOwned + PartialEq + fmt::Debug {
    type Kind: DeserializeOwned + fmt::Debug + Clone + PartialEq;

    fn new(unlocked: &[&Advancement<Self>]) -> Self;
    fn filter_base(a: &Advancement<Self>) -> bool;
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(bound(deserialize = ""))]
pub struct Advancement<S: AdvancementSum> {
    pub kind: S::Kind,
    pub xp: u64,
    pub art: String,
    pub title: String,
    pub description: String,
    pub achiever_title: String,
}
#[derive(Deserialize, Debug, Clone)]
#[serde(bound(deserialize = ""))]
pub struct AdvancementSet<S: AdvancementSum> {
    pub base: Advancement<S>,
    pub rest: Vec<Advancement<S>>,
}
#[allow(dead_code)]
impl<S: AdvancementSum> AdvancementSet<S> {
    pub fn all(&self) -> impl Iterator<Item = &Advancement<S>> {
        std::iter::once(&self.base).chain(self.rest.iter())
    }
    pub fn unlocked(&self, xp: u64) -> impl Iterator<Item = &Advancement<S>> {
        std::iter::once(&self.base).chain(self.rest.iter().take(self.current_position(xp)))
    }

    pub fn get(&self, index: usize) -> Option<&Advancement<S>> {
        if index == 0 {
            Some(&self.base)
        } else {
            self.rest.get(index - 1)
        }
    }

    pub fn increment_xp(&self, xp: &mut u64) -> Option<&Advancement<S>> {
        *xp += 1;
        self.next(*xp - 1)
            .filter(|&x| self.next(*xp).map(|n| *x != *n).unwrap_or(false))
    }

    pub fn sum<'a>(
        &'a self,
        xp: u64,
        extra_advancements: impl Iterator<Item = &'a Advancement<S>>,
    ) -> S {
        S::new(
            &self
                .unlocked(xp)
                .filter(|&x| S::filter_base(x))
                .chain(extra_advancements)
                .collect::<Vec<_>>(),
        )
    }

    pub fn raw_sum(&self, xp: u64) -> S {
        S::new(&self.unlocked(xp).collect::<Vec<_>>())
    }

    pub fn max<'a>(&'a self, extra_advancements: impl Iterator<Item = &'a Advancement<S>>) -> S {
        S::new(
            &self
                .all()
                .filter(|&x| S::filter_base(x))
                .chain(extra_advancements)
                .collect::<Vec<_>>(),
        )
    }

    pub fn current(&self, xp: u64) -> &Advancement<S> {
        self.get(self.current_position(xp)).unwrap_or(&self.base)
    }

    pub fn next(&self, xp: u64) -> Option<&Advancement<S>> {
        self.get(self.current_position(xp) + 1)
    }

    pub fn current_position(&self, xp: u64) -> usize {
        let mut state = 0;
        self.all()
            .position(|x| {
                state += x.xp;
                state > xp
            })
            .unwrap_or(self.rest.len() + 1)
            .checked_sub(1)
            .unwrap_or(0)
    }
}

#[test]
fn upgrade_increase() {
    for arch in CONFIG.plant_archetypes.iter() {
        let adv = &arch.advancements;
        let last = adv.rest.last().unwrap();
        for xp in 0..last.xp {
            assert!(
                adv.current(xp).xp <= xp,
                "when xp is {} for {} the current advancement has more xp({})",
                xp,
                arch.name,
                adv.current(xp).xp
            );
        }
    }
}

#[test]
/// In the CONFIG, you can specify the names of archetypes.
/// If you're Rishi, you might spell one of those names wrong.
/// This test helps you make sure you didn't do that.
fn archetype_name_matches() {
    for a in CONFIG.possession_archetypes.iter() {
        match &a.kind {
            ArchetypeKind::Seed(sa) => assert!(
                CONFIG.find_plant(&sa.grows_into).is_ok(),
                "seed archetype {:?} claims it grows into unknown plant archetype {:?}",
                a.name,
                sa.grows_into,
            ),
            _ => {}
        }
    }

    for arch in CONFIG.plant_archetypes.iter().cloned() {
        for adv in arch.advancements.all() {
            use PlantAdvancementKind::*;

            match &adv.kind {
                Yield(resources) => {
                    for (_, item_name) in resources.iter() {
                        assert!(
                            CONFIG.find_possession(item_name).is_ok(),
                            "Yield advancement {:?} for plant {:?} includes unknown resource {:?}",
                            adv.title,
                            arch.name,
                            item_name,
                        )
                    }
                }
                Craft(recipes) => {
                    for Recipe { makes, needs, .. } in recipes.iter() {
                        assert!(
                            CONFIG.find_possession(makes).is_ok(),
                            "Crafting advancement {:?} for plant {:?} produces unknown resource {:?}",
                            adv.title,
                            arch.name,
                            makes,
                        );
                        for (_, resource) in needs.iter() {
                            assert!(
                                CONFIG.find_possession(resource).is_ok(),
                                "Crafting advancement {:?} for plant {:?} uses unknown resource {:?} in recipe for {:?}",
                                adv.title,
                                arch.name,
                                resource,
                                makes
                            )
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
