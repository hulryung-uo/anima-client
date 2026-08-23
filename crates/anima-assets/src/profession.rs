//! `Prof.txt` profession table (ClassicUO `ProfessionLoader`).
//!
//! Each `Begin`…`End` block is a profession or category. The `Desc` integer is
//! the byte `CreateCharacter` `0xF8` sends as profession (0 = custom/Advanced).
//! Skill names are resolved against `skills.mul` (plus ClassicUO's
//! `SkillEntry.HardCodedName` aliases, because Prof.txt says "Blacksmith" and
//! "Evaluate Intelligence" while the mul says "Blacksmithy" /
//! "Evaluating Intelligence").

use std::path::Path;

use crate::skills::Skills;

/// One selectable profession (or the leftover Advanced custom row).
#[derive(Debug, Clone)]
pub struct Profession {
    pub name: String,
    pub true_name: String,
    /// Cliloc for the on-screen name (`NameId`).
    pub name_id: u32,
    /// Cliloc for the description (`DescId`).
    pub desc_id: u32,
    /// The `0xF8` profession byte (`Desc`). 0 for Advanced/custom.
    pub desc: i32,
    pub top_level: bool,
    pub gump: u16,
    pub is_category: bool,
    /// Up to four `(skill_id, value)` pairs. Unused slots are `(0, 0)`.
    pub skills: [(u8, u8); 4],
    /// STR, DEX, INT.
    pub stats: [u8; 3],
}

impl Profession {
    /// The leftover "Advanced" row ClassicUO always appends (`ProfessionLoader`
    /// after parsing Prof.txt): profession byte 0, localisation 1061176/1061226,
    /// gump 5545.
    pub fn advanced() -> Self {
        Self {
            name: "Advanced".into(),
            true_name: "advanced".into(),
            name_id: 1_061_176,
            desc_id: 1_061_226,
            desc: -1,
            top_level: true,
            gump: 5545,
            is_category: false,
            skills: [(0, 0); 4],
            stats: [60, 15, 15],
        }
    }

    pub fn to_json(&self) -> String {
        let skills: Vec<String> = self
            .skills
            .iter()
            .filter(|(_, v)| *v > 0)
            .map(|(id, v)| format!("{{\"id\":{id},\"value\":{v}}}"))
            .collect();
        format!(
            "{{\"name\":{},\"trueName\":{},\"nameId\":{},\"descId\":{},\"profession\":{},\
             \"topLevel\":{},\"gump\":{},\"category\":{},\"skills\":[{}],\
             \"str\":{},\"dex\":{},\"int\":{}}}",
            json_str(&self.name),
            json_str(&self.true_name),
            self.name_id,
            self.desc_id,
            self.desc.max(0),
            u8::from(self.top_level),
            self.gump,
            u8::from(self.is_category),
            skills.join(","),
            self.stats[0],
            self.stats[1],
            self.stats[2],
        )
    }
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Parsed `Prof.txt` plus the Advanced row.
#[derive(Debug, Clone, Default)]
pub struct Professions {
    pub entries: Vec<Profession>,
}

impl Professions {
    pub fn open(resource_dir: impl AsRef<Path>, skills: Option<&Skills>) -> std::io::Result<Self> {
        let path = resource_dir.as_ref().join("Prof.txt");
        let text = std::fs::read_to_string(&path)?;
        if text.len() > 0x100_000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Prof.txt exceeds 1MB",
            ));
        }
        Ok(Self::parse(&text, skills))
    }

    pub fn parse(text: &str, skills: Option<&Skills>) -> Self {
        let mut entries = parse_blocks(text, skills);
        entries.push(Profession::advanced());
        Self { entries }
    }

    pub fn to_json(&self) -> String {
        let body = self
            .entries
            .iter()
            .filter(|p| p.top_level && !p.is_category)
            .map(Profession::to_json)
            .collect::<Vec<_>>()
            .join(",");
        format!("[{body}]")
    }

    /// Look up by TrueName (case-insensitive) or by the `Desc` profession byte.
    pub fn get(&self, name_or_id: &str) -> Option<&Profession> {
        if let Ok(id) = name_or_id.parse::<i32>() {
            if id == 0 {
                return self
                    .entries
                    .iter()
                    .find(|p| p.desc < 0 || p.true_name.eq_ignore_ascii_case("advanced"));
            }
            return self.entries.iter().find(|p| p.desc == id);
        }
        let n = name_or_id.trim();
        self.entries
            .iter()
            .find(|p| p.true_name.eq_ignore_ascii_case(n) || p.name.eq_ignore_ascii_case(n))
    }
}

fn parse_blocks(text: &str, skills: Option<&Skills>) -> Vec<Profession> {
    let mut out = Vec::new();
    let mut cur: Option<ProfessionBuilder> = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let (key, rest) = split_key(line);
        match key.to_ascii_lowercase().as_str() {
            "begin" => cur = Some(ProfessionBuilder::default()),
            "end" => {
                if let Some(b) = cur.take() {
                    out.push(b.finish());
                }
            }
            other => {
                if let Some(b) = cur.as_mut() {
                    b.apply(other, rest, skills);
                }
            }
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let hash = line.find('#').unwrap_or(line.len());
    let semi = line.find(';').unwrap_or(line.len());
    &line[..hash.min(semi)]
}

fn split_key(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    }
}

#[derive(Default)]
struct ProfessionBuilder {
    name: String,
    true_name: String,
    name_id: u32,
    desc_id: u32,
    desc: i32,
    top_level: bool,
    gump: u16,
    is_category: bool,
    skills: [(u8, u8); 4],
    skill_n: usize,
    stats: [u8; 3],
}

impl ProfessionBuilder {
    fn apply(&mut self, key: &str, rest: &str, skills: Option<&Skills>) {
        match key.to_ascii_lowercase().as_str() {
            "name" => self.name = unquote(rest),
            "truename" => self.true_name = unquote(rest),
            "nameid" => self.name_id = parse_u32(rest),
            "descid" => self.desc_id = parse_u32(rest),
            "desc" => {
                self.desc = rest
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
            }
            "toplevel" => self.top_level = rest.eq_ignore_ascii_case("true"),
            "gump" => self.gump = parse_u32(rest) as u16,
            "type" => self.is_category = rest.eq_ignore_ascii_case("category"),
            "skill" => self.push_skill(rest, skills),
            "stat" => self.push_stat(rest),
            _ => {}
        }
    }

    fn push_skill(&mut self, rest: &str, skills: Option<&Skills>) {
        if self.skill_n >= 4 {
            return;
        }
        let (name, value) = split_skill(rest);
        let Some(id) = resolve_skill(name, skills) else {
            return;
        };
        self.skills[self.skill_n] = (id, value.min(50));
        self.skill_n += 1;
    }

    fn push_stat(&mut self, rest: &str) {
        let mut it = rest.split_whitespace();
        let which = it.next().unwrap_or("").to_ascii_lowercase();
        let value = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        match which.as_str() {
            "str" | "strength" => self.stats[0] = value,
            "dex" | "dexterity" => self.stats[1] = value,
            "int" | "intelligence" => self.stats[2] = value,
            _ => {}
        }
    }

    fn finish(self) -> Profession {
        let name = self.name;
        Profession {
            name: name.clone(),
            true_name: if self.true_name.is_empty() {
                name
            } else {
                self.true_name
            },
            name_id: self.name_id,
            desc_id: self.desc_id,
            desc: self.desc,
            top_level: self.top_level,
            gump: self.gump,
            is_category: self.is_category,
            skills: self.skills,
            stats: self.stats,
        }
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        inner.to_string()
    } else {
        s.split_whitespace().next().unwrap_or("").to_string()
    }
}

fn parse_u32(s: &str) -> u32 {
    s.split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn split_skill(rest: &str) -> (&str, u8) {
    let rest = rest.trim();
    if let Some(i) = rest.rfind(char::is_whitespace) {
        let (name, val) = rest.split_at(i);
        (name.trim(), val.trim().parse().unwrap_or(0))
    } else {
        (rest, 0)
    }
}

/// ClassicUO `SkillEntry.HardCodedName` aliases used when Prof.txt's skill
/// token disagrees with `skills.mul`'s display name.
fn hardcoded_alias(name: &str) -> Option<u8> {
    let n = name.replace([' ', '/'], "").to_ascii_lowercase();
    Some(match n.as_str() {
        "alchemy" => 0,
        "anatomy" => 1,
        "animallore" => 2,
        "itemid" | "itemidentification" => 3,
        "armslore" => 4,
        "parrying" => 5,
        "begging" => 6,
        "blacksmith" | "blacksmithy" => 7,
        "bowcraft" | "bowcraftfletching" | "fletching" => 8,
        "peacemaking" => 9,
        "camping" => 10,
        "carpentry" => 11,
        "cartography" => 12,
        "cooking" => 13,
        "detecthidden" | "detectinghidden" => 14,
        "enticement" | "discordance" => 15,
        "evaluateintelligence" | "evaluatingintelligence" | "evalint" => 16,
        "healing" => 17,
        "fishing" => 18,
        "forensicevaluation" | "forensiceval" => 19,
        "herding" => 20,
        "hiding" => 21,
        "provocation" => 22,
        "inscription" => 23,
        "lockpicking" => 24,
        "magery" => 25,
        "resistingspells" | "magicresist" => 26,
        "tactics" => 27,
        "snooping" => 28,
        "musicianship" | "musicanship" => 29,
        "poisoning" => 30,
        "archery" => 31,
        "spiritspeak" => 32,
        "stealing" => 33,
        "tailoring" => 34,
        "animaltaming" => 35,
        "tasteidentification" => 36,
        "tinkering" => 37,
        "tracking" => 38,
        "veterinary" => 39,
        "swordsmanship" => 40,
        "macefighting" => 41,
        "fencing" => 42,
        "wrestling" => 43,
        "lumberjacking" => 44,
        "mining" => 45,
        "meditation" => 46,
        "stealth" => 47,
        "removetrap" => 48,
        "necromancy" => 49,
        "focus" => 50,
        "chivalry" => 51,
        "bushido" => 52,
        "ninjitsu" => 53,
        "spellweaving" => 54,
        "mysticism" => 55,
        "imbuing" => 56,
        "throwing" => 57,
        _ => return None,
    })
}

fn resolve_skill(name: &str, skills: Option<&Skills>) -> Option<u8> {
    let name = name.trim();
    if let Some(skills) = skills {
        if let Some(s) = skills
            .entries
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
        {
            return Some(s.id as u8);
        }
    }
    hardcoded_alias(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Begin
	Name			Warrior
	TrueName		"Warrior"
	NameId			1061180
	DescId			1061230
	Desc			1
	TopLevel		true
	Gump			5577
	Type			Profession
	Skill			Tactics			30
	Skill			Healing			30
	Skill			Swordsmanship		30
	Skill			Anatomy		30
	Stat			Str			45
	Stat			Dex			35
	Stat			Int			10
End
"#;

    #[test]
    fn parses_warrior_and_appends_advanced() {
        let p = Professions::parse(SAMPLE, None);
        assert_eq!(p.entries.len(), 2);
        let w = p.get("Warrior").expect("warrior");
        assert_eq!(w.desc, 1);
        assert_eq!(w.gump, 5577);
        assert_eq!(w.stats, [45, 35, 10]);
        assert_eq!(w.skills[0], (27, 30)); // tactics
        assert_eq!(w.skills[2], (40, 30)); // swordsmanship
        assert_eq!(p.get("0").map(|a| a.name.as_str()), Some("Advanced"));
    }

    #[test]
    fn blacksmith_alias_resolves_to_skill_7() {
        assert_eq!(hardcoded_alias("Blacksmith"), Some(7));
        assert_eq!(hardcoded_alias("Evaluate Intelligence"), Some(16));
    }
}
