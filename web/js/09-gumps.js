// ---- paperdoll + container windows (HTML "gump" overlays over the canvas) ----
// These are plain DOM panels (divs over the PixiJS canvas), not sprites — gump-style
// chrome is far simpler/safer in HTML than in the render loop. They READ `scene`
// each poll (refresh*) and only act via sendInput(); they never touch movement/render.
const EQUIP_SLOTS = {
  1: "Right Hand", 2: "Left Hand", 3: "Shoes", 4: "Pants", 5: "Shirt", 6: "Head",
  7: "Gloves", 8: "Ring", 9: "Talisman", 10: "Neck", 11: "Hair", 12: "Waist",
  13: "Torso", 14: "Bracelet", 15: "Face", 16: "Beard", 17: "Tunic", 18: "Earrings", 19: "Arms",
  20: "Cloak", 21: "Backpack", 22: "Robe", 23: "OuterLegs", 24: "InnerLegs",
};
const BACKPACK_LAYER = 21;
// Paperdoll draw order (back → front), by UO layer number — ClassicUO
// PaperDollInteractable._layerOrder. The worn item's paperdoll gump is its
// AnimID + a gender offset; each gump is a full doll-canvas image stacked at the
// same origin. Weapons (1/2) are included so held items show on the doll.
const PAPERDOLL_ORDER = [20, 5, 4, 3, 24, 19, 13, 17, 8, 14, 15, 7, 23, 22, 12, 10, 11, 16, 18, 6, 1, 2, 9];
const MALE_GUMP_OFFSET = 50000, FEMALE_GUMP_OFFSET = 60000;

// Bring a gump window to the top by moving it to the end of <body> (all gumps share
// the same z-index, so DOM order decides paint order). Keeps them below the modal
// world map (z 20) and above the HUD.
function bringToFront(el) { document.body.appendChild(el); }

// Drag a window by its title bar; clamp so it never fully leaves the viewport.
function makeDraggable(win, handle, onMove) {
  handle.addEventListener("mousedown", (e) => {
    if (e.target.classList.contains("gump-close")) return; // let the ✕ click through
    e.preventDefault();
    bringToFront(win);
    const r = win.getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    const move = (ev) => {
      const x = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - dx));
      const y = Math.max(0, Math.min(window.innerHeight - 24, ev.clientY - dy));
      win.style.left = x + "px"; win.style.top = y + "px"; win.style.right = "auto";
      if (onMove) onMove(x, y);
    };
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}

// --- paperdoll (toggled by the 'P' key; ✕/Esc close) ---
// pdTarget: null = our own doll; a serial = another mobile's doll (double-click an
// NPC/player to inspect their equipment, ClassicUO-style).
let paperdollOn = false;
let pdTarget = null;
function togglePaperdoll() {
  // 'P' always shows OUR doll (switch back from any inspected mobile).
  if (paperdollOn && pdTarget == null) { closePaperdoll(); return; }
  pdTarget = null;
  paperdollOn = true;
  const pd = document.getElementById("paperdoll");
  pd.classList.add("on"); pd._sig = null;
  refreshPaperdoll();
}
// Open another mobile's paperdoll (double-clicked in the world).
function openMobilePaperdoll(serial) {
  pdTarget = serial >>> 0;
  paperdollOn = true;
  const pd = document.getElementById("paperdoll");
  pd.classList.add("on"); pd._sig = null;
  refreshPaperdoll();
}
function closePaperdoll() {
  paperdollOn = false;
  pdTarget = null;
  document.getElementById("paperdoll").classList.remove("on");
}
// --- weapon special-ability bar (bottom-left; click to arm/disarm) ---
// Two buttons = the equipped weapon's primary/secondary moves. Clicking sends
// 0xD7 UseCombatAbility (Action::UseAbility → `sendInput("ability:"+id)`) with the
// move's `Ability` enum id; clicking the armed one again disarms (sends 0). The
// server actually arms/disarms the next swing — we just mirror the highlight.
// Names + weapon→ability table ported from ClassicUO Game/Data/Ability.cs.
const ABILITY_NAMES = {
  1: "Armor Ignore", 2: "Bleed Attack", 3: "Concussion Blow", 4: "Crushing Blow",
  5: "Disarm", 6: "Dismount", 7: "Double Strike", 8: "Infectious Strike",
  9: "Mortal Strike", 10: "Moving Shot", 11: "Paralyzing Blow", 12: "Shadow Strike",
  13: "Whirlwind Attack", 14: "Riding Swipe", 15: "Frenzied Whirlwind", 16: "Block",
  17: "Defense Mastery", 18: "Nerve Strike", 19: "Talon Strike", 20: "Feint",
  21: "Dual Wield", 22: "Double Shot", 23: "Armor Pierce", 24: "Bladeweave",
  25: "Force Arrow", 26: "Lightning Arrow", 27: "Psychic Attack", 28: "Serpent Arrow",
  29: "Force of Nature", 30: "Infused Throw", 31: "Mystic Arc",
};
// weapon graphic → [primaryAbilityId, secondaryAbilityId]
const WEAPON_ABILITIES = {
  0x0901:[10,30], 0x0902:[8,12], 0x0905:[7,9], 0x0906:[4,6], 0x090C:[2,9], 0x0DF0:[13,11],
  0x0DF1:[13,11], 0x0DF2:[6,5], 0x0DF3:[6,5], 0x0DF4:[6,5], 0x0DF5:[6,5], 0x0E81:[4,5],
  0x0E82:[4,5], 0x0E85:[7,5], 0x0E86:[7,5], 0x0E87:[2,6], 0x0E88:[2,6], 0x0E89:[7,3],
  0x0E8A:[7,3], 0x0EC2:[2,8], 0x0EC3:[2,8], 0x0EC4:[12,2], 0x0EC5:[12,2], 0x0F43:[1,5],
  0x0F44:[1,5], 0x0F45:[2,9], 0x0F46:[2,9], 0x0F47:[2,3], 0x0F48:[2,3], 0x0F49:[4,6],
  0x0F4A:[4,6], 0x0F4B:[7,13], 0x0F4C:[7,13], 0x0F4D:[11,6], 0x0F4E:[11,6], 0x0F4F:[3,9],
  0x0F50:[3,9], 0x0F51:[8,12], 0x0F52:[8,12], 0x0F5C:[3,5], 0x0F5D:[3,5], 0x0F5E:[4,1],
  0x0F5F:[4,1], 0x0F60:[1,3], 0x0F61:[1,3], 0x0F62:[1,11], 0x0F63:[1,11], 0x0FB5:[4,12],
  0x13AF:[1,2], 0x13B0:[1,2], 0x13B1:[11,9], 0x13B2:[11,9], 0x13B3:[12,6], 0x13B4:[12,6],
  0x13B6:[7,11], 0x13B7:[7,11], 0x13B8:[7,11], 0x13B9:[11,4], 0x13BA:[11,4], 0x13FD:[10,6],
  0x13E3:[4,12], 0x13F6:[8,5], 0x13F8:[3,29], 0x13FB:[13,2], 0x13FF:[7,1], 0x1401:[1,8],
  0x1402:[12,9], 0x1403:[12,9], 0x1404:[2,5], 0x1405:[2,5], 0x1406:[4,9], 0x1407:[4,9],
  0x1438:[13,4], 0x1439:[13,4], 0x143A:[7,3], 0x143B:[7,3], 0x143C:[1,9], 0x143D:[1,9],
  0x143E:[13,3], 0x143F:[13,3], 0x1440:[2,12], 0x1441:[2,12], 0x1442:[7,12], 0x1443:[7,12],
  0x26BA:[2,11], 0x26BB:[11,9], 0x26BC:[4,9], 0x26BD:[1,6], 0x26BE:[11,8], 0x26BF:[7,8],
  0x26C0:[6,3], 0x26C1:[7,9], 0x26C2:[1,10], 0x26C3:[7,10], 0x26C4:[2,11], 0x26C5:[11,9],
  0x26C6:[4,9], 0x26C7:[1,6], 0x26C8:[11,8], 0x26C9:[7,8], 0x26CA:[6,3], 0x26CB:[7,9],
  0x26CC:[1,10], 0x26CD:[7,10], 0x26CE:[13,5], 0x26CF:[13,5], 0x27A2:[4,14], 0x27A3:[20,16],
  0x27A4:[15,7], 0x27A5:[23,22], 0x27A6:[15,4], 0x27A7:[17,15], 0x27A8:[20,18], 0x27A9:[20,7],
  0x27AA:[5,11], 0x27AB:[21,19], 0x27AD:[13,17], 0x27AE:[16,20], 0x27AF:[16,23], 0x27ED:[4,14],
  0x27EE:[20,16], 0x27EF:[15,7], 0x27F0:[23,22], 0x27F1:[15,4], 0x27F2:[17,15], 0x27F3:[20,18],
  0x27F4:[20,7], 0x27F5:[5,11], 0x27F6:[21,19], 0x27F8:[13,17], 0x27F9:[16,20], 0x27FA:[16,23],
  0x2D1E:[25,28], 0x2D1F:[26,27], 0x2D20:[27,2], 0x2D21:[8,12], 0x2D22:[20,1], 0x2D23:[5,24],
  0x2D24:[3,4], 0x2D25:[16,29], 0x2D26:[5,24], 0x2D27:[13,24], 0x2D28:[5,4], 0x2D29:[17,24],
  0x2D2A:[25,28], 0x2D2B:[26,27], 0x2D2C:[27,2], 0x2D2D:[8,12], 0x2D2E:[20,1], 0x2D2F:[5,24],
  0x2D30:[3,4], 0x2D31:[16,29], 0x2D32:[5,24], 0x2D33:[13,24], 0x2D34:[5,4], 0x2D35:[17,24],
  0x4067:[31,3], 0x08FD:[7,8], 0x4068:[7,8], 0x406B:[1,9], 0x406C:[10,30], 0x0904:[7,5],
  0x406D:[7,5], 0x0903:[1,5], 0x406E:[1,5], 0x08FE:[2,11], 0x4072:[2,11], 0x090B:[4,3],
  0x4074:[4,3], 0x0908:[13,6], 0x4075:[13,6], 0x4076:[1,9], 0x48AE:[2,8], 0x48B0:[2,3],
  0x48B3:[4,6], 0x48B2:[4,6], 0x48B5:[11,6], 0x48B4:[11,6], 0x48B7:[8,5], 0x48B6:[8,5],
  0x48B9:[3,11], 0x48B8:[3,11], 0x48BB:[7,1], 0x48BA:[7,1], 0x48BD:[1,8], 0x48BC:[1,8],
  0x48BF:[2,5], 0x48BE:[2,5], 0x48CB:[6,3], 0x48CA:[6,3], 0x0481:[13,4], 0x48C0:[13,4],
  0x48C3:[7,3], 0x48C2:[7,3], 0x48C5:[2,11], 0x48C4:[2,11], 0x48C7:[11,9], 0x48C6:[11,9],
  0x48C9:[11,8], 0x48C8:[11,8], 0x48CC:[20,16], 0x48CD:[20,16], 0x48CF:[21,19], 0x48CE:[21,19],
  0x48D1:[20,7], 0x48D0:[20,7], 0xA289:[3,13], 0xA291:[3,13], 0xA28A:[23,13], 0xA292:[23,13],
  0xA28B:[2,13], 0xA293:[2,13], 0x08FF:[31,3], 0x0900:[1,11], 0x090A:[1,9], 0xAEA5:[7,1],
  0xAEB4:[7,1], 0xAEC3:[7,1], 0xAED2:[7,1], 0xAEA4:[7,13], 0xAEB3:[7,13], 0xAEC2:[7,13],
  0xAED1:[7,13],
};
const WEAPON_LAYERS = [1, 2]; // right hand / left hand (two-handed weapons sit on 1)
let armedAbility = 0;    // the ability id armed right now (0 = none)
let armedLocalUntil = 0; // trust our own value, not the server's, until this timestamp
// How long a click's optimistic highlight outranks `scene.armedAbility`.
// `scene.json` is POLLED every 150ms (see `poll`), and the snapshot answering a
// click was very likely BUILT BEFORE that click reached the server — the exact
// hazard `web/dialogs.js` handles for dialog windows (DESIGN.md D11). Reading
// the server value unconditionally would therefore blank the highlight for one
// poll on every arm. Three polls is a generous round trip and still an upper
// bound on how long a genuine server clear can be hidden.
const ARM_ECHO_GRACE_MS = 500;
// Reconcile the bar with the server's arm (`scene.armedAbility`, driven by the
// 0xBF/0x21 ClearWeaponAbility the server sends once the move is spent, missed,
// refused for mana, or invalidated by a weapon change).
//
// Outside the grace window the server is simply believed — no change detection,
// no "did we cause this". That matters because a swing can resolve inside a
// single poll interval: an arm that is spent before its own echo ever lands
// would leave a diff-based reconciler showing a move the player no longer has.
function syncArmedAbility() {
  if (performance.now() < armedLocalUntil) return;
  const served = (scene && scene.armedAbility) | 0;
  if (served === armedAbility) return;
  armedAbility = served;
  refreshAbilities(true);
}
// Find the equipped weapon's [primaryId, secondaryId], or null if none/unmapped.
function equippedWeaponAbilities() {
  const p = scene && scene.player;
  if (!p || !p.equip) return null;
  for (const layer of WEAPON_LAYERS) {
    const w = p.equip.find((e) => (e.layer | 0) === layer);
    if (w) return WEAPON_ABILITIES[w.g & 0xFFFF] || "unknown";
  }
  return null;
}
function clickAbility(id) {
  if (!id) return;
  // ClassicUO toggle: clicking the armed move disarms (send 0), else arm it.
  if (armedAbility === id) { armedAbility = 0; sendInput("ability:0"); }
  else { armedAbility = id; sendInput("ability:" + id); }
  armedLocalUntil = performance.now() + ARM_ECHO_GRACE_MS;
  refreshAbilities(true);
}
// Rebuild the two-button bar from the equipped weapon (called each poll).
function refreshAbilities(force) {
  const bar = document.getElementById("abilities");
  if (!bar) return;
  // Weapon special abilities are an AOS feature. Show the bar only when the server
  // advertised AOS in SupportedFeatures (0xB9) AND the user hasn't hidden it in
  // Options — a T2A/pre-AOS shard has no such moves. (Note: a shard that enables
  // Core.AOS server-side, e.g. for OPL tooltips, will advertise AOS=true here.)
  // T2A explicitly hides the bar regardless of the AOS flag (see `T2A` const).
  if (T2A || !(scene && scene.aos) || !settings.abilities) { bar.style.display = "none"; return; }
  bar.style.display = "flex";
  const ab = equippedWeaponAbilities();
  // ab: null = no weapon → generic ids 0/1 ; "unknown" = weapon not in table ;
  // [p,s] = real moves. (ids 0/1 let the server pick based on the equipped weapon.)
  let prim, sec, primName, secName;
  if (Array.isArray(ab)) {
    [prim, sec] = ab;
    primName = ABILITY_NAMES[prim] || "Primary";
    secName = ABILITY_NAMES[sec] || "Secondary";
  } else {
    prim = 0; sec = 1; primName = "Primary"; secName = "Secondary";
  }
  const sig = `${prim}:${sec}:${armedAbility}`;
  if (!force && bar._sig === sig) return;
  bar._sig = sig;
  bar.innerHTML =
    `<div class="abil-hdr">Weapon Abilities</div>` +
    `<div class="abil-btn${armedAbility && armedAbility === prim ? " armed" : ""}" data-id="${prim}">` +
    `<span>${primName}</span><span class="ak">PRI</span></div>` +
    `<div class="abil-btn${armedAbility && armedAbility === sec ? " armed" : ""}" data-id="${sec}">` +
    `<span>${secName}</span><span class="ak">SEC</span></div>`;
  for (const el of bar.querySelectorAll(".abil-btn"))
    el.addEventListener("click", () => clickAbility(Number(el.dataset.id)));
}

// --- spellbook (all schools, toggled by the 'K' key; ✕/Esc close) ---
// The cast packet (0xBF/0x1C, Action::CastSpell) takes a GLOBAL spell id, so every
// school is the same mechanism — `sendInput("cast:" + id)`. Ids/names ported from
// ClassicUO src/ClassicUO.Client/Game/Data/Spells*.cs.
// 64 Magery spells, ids 1..64 = (circle-1)*8 + position. Names in id order.
const MAGERY_SPELLS = [
  "Clumsy", "Create Food", "Feeblemind", "Heal", "Magic Arrow", "Night Sight",
  "Reactive Armor", "Weaken", "Agility", "Cunning", "Cure", "Harm", "Magic Trap",
  "Magic Untrap", "Protection", "Strength", "Bless", "Fireball", "Magic Lock",
  "Poison", "Telekinesis", "Teleport", "Unlock", "Wall of Stone", "Arch Cure",
  "Arch Protection", "Curse", "Fire Field", "Greater Heal", "Lightning",
  "Mana Drain", "Recall", "Blade Spirits", "Dispel Field", "Incognito",
  "Magic Reflection", "Mind Blast", "Paralyze", "Poison Field", "Summon Creature",
  "Dispel", "Energy Bolt", "Explosion", "Invisibility", "Mark", "Mass Curse",
  "Paralyze Field", "Reveal", "Chain Lightning", "Energy Field", "Flamestrike",
  "Gate Travel", "Mana Vampire", "Mass Dispel", "Meteor Swarm", "Polymorph",
  "Earthquake", "Energy Vortex", "Resurrection", "Air Elemental", "Summon Daemon",
  "Earth Elemental", "Fire Elemental", "Water Elemental",
];
// Other schools as [globalId, name] (ported from Spells*.cs).
const NECROMANCY_SPELLS = [
  [101, "Animate Dead"], [102, "Blood Oath"], [103, "Corpse Skin"], [104, "Curse Weapon"],
  [105, "Evil Omen"], [106, "Horrific Beast"], [107, "Lich Form"], [108, "Mind Rot"],
  [109, "Pain Spike"], [110, "Poison Strike"], [111, "Strangle"], [112, "Summon Familiar"],
  [113, "Vampiric Embrace"], [114, "Vengeful Spirit"], [115, "Wither"], [116, "Wraith Form"],
  [117, "Exorcism"],
];
const CHIVALRY_SPELLS = [
  [201, "Cleanse by Fire"], [202, "Close Wounds"], [203, "Consecrate Weapon"],
  [204, "Dispel Evil"], [205, "Divine Fury"], [206, "Enemy of One"], [207, "Holy Light"],
  [208, "Noble Sacrifice"], [209, "Remove Curse"], [210, "Sacred Journey"],
];
const BUSHIDO_SPELLS = [
  [401, "Honorable Execution"], [402, "Confidence"], [403, "Evasion"],
  [404, "Counter Attack"], [405, "Lightning Strike"], [406, "Momentum Strike"],
];
const NINJITSU_SPELLS = [
  [501, "Focus Attack"], [502, "Death Strike"], [503, "Animal Form"], [504, "Ki Attack"],
  [505, "Surprise Attack"], [506, "Backstab"], [507, "Shadowjump"], [508, "Mirror Image"],
];
const SPELLWEAVING_SPELLS = [
  [601, "Arcane Circle"], [602, "Gift of Renewal"], [603, "Immolating Weapon"],
  [604, "Attunement"], [605, "Thunderstorm"], [606, "Nature's Fury"], [607, "Summon Fey"],
  [608, "Summon Fiend"], [609, "Reaper Form"], [610, "Wildfire"], [611, "Essence of Wind"],
  [612, "Dryad Allure"], [613, "Ethereal Voyage"], [614, "Word of Death"],
  [615, "Gift of Life"], [616, "Arcane Empowerment"],
];
const MYSTICISM_SPELLS = [
  [678, "Nether Bolt"], [679, "Healing Stone"], [680, "Purge Magic"], [681, "Enchant"],
  [682, "Sleep"], [683, "Eagle Strike"], [684, "Animated Weapon"], [685, "Stone Form"],
  [686, "Spell Trigger"], [687, "Mass Sleep"], [688, "Cleansing Winds"], [689, "Bombard"],
  [690, "Spell Plague"], [691, "Hail Storm"], [692, "Nether Cyclone"], [693, "Rising Colossus"],
];
const MASTERY_SPELLS = [
  [701, "Inspire"], [702, "Invigorate"], [703, "Resilience"], [704, "Perseverance"],
  [705, "Tribulation"], [706, "Despair"], [707, "Death Ray"], [708, "Ethereal Burst"],
  [709, "Nether Blast"], [710, "Mystic Weapon"], [711, "Command Undead"], [712, "Conduit"],
  [713, "Mana Shield"], [714, "Summon Reaper"], [715, "Enchanted Summoning"],
  [716, "Anticipate Hit"], [717, "Warcry"], [718, "Intuition"], [719, "Rejuvenate"],
  [720, "Holy Fist"], [721, "Shadow"], [722, "White Tiger Form"], [723, "Flaming Shot"],
  [724, "Playing The Odds"], [725, "Thrust"], [726, "Pierce"], [727, "Stagger"],
  [728, "Toughness"], [729, "Onslaught"], [730, "Focused Eye"], [731, "Elemental Fury"],
  [732, "Called Shot"], [733, "Warrior's Gifts"], [734, "Shield Bash"], [735, "Bodyguard"],
  [736, "Heighten Senses"], [737, "Tolerance"], [738, "Injected Strike"], [739, "Potency"],
  [740, "Rampage"], [741, "Fists of Fury"], [742, "Knockout"], [743, "Whispering"],
  [744, "Combat Training"], [745, "Boarding"],
];
// Each school renders as its own native ClassicUO book gump. `book` = the book
// background gump id; `iconStart` = the gump id of the school's first spell icon
// (icon for the k-th spell, k 0-based, = iconStart + k). Ids decimal (the gump
// endpoint /gump/<id>.png wants decimal). Source: SpellbookGump.cs GetBookInfo.
// `spells` is normalised to [globalId, name] for every school (Magery built below).
const MAGERY_PAIRS = MAGERY_SPELLS.map((name, i) => [i + 1, name]);
// Power words (mantra) + reagents per Magery spell, keyed by name. Source: ServUO
// Scripts/Spells/{First..Eighth}/*.cs SpellInfo (name, mantra, Reagent.*).
const MAGERY_INFO = {
  "Clumsy": ["Uus Jux", "Bloodmoss, Nightshade"],
  "Create Food": ["In Mani Ylem", "Garlic, Ginseng, Mandrake Root"],
  "Feeblemind": ["Rel Wis", "Ginseng, Nightshade"],
  "Heal": ["In Mani", "Garlic, Ginseng, Spider's Silk"],
  "Magic Arrow": ["In Por Ylem", "Sulfurous Ash"],
  "Night Sight": ["In Lor", "Sulfurous Ash, Spider's Silk"],
  "Reactive Armor": ["Flam Sanct", "Garlic, Spider's Silk, Sulfurous Ash"],
  "Weaken": ["Des Mani", "Garlic, Nightshade"],
  "Agility": ["Ex Uus", "Bloodmoss, Mandrake Root"],
  "Cunning": ["Uus Wis", "Mandrake Root, Nightshade"],
  "Cure": ["An Nox", "Garlic, Ginseng"],
  "Harm": ["An Mani", "Nightshade, Spider's Silk"],
  "Magic Trap": ["In Jux", "Garlic, Spider's Silk, Sulfurous Ash"],
  "Magic Untrap": ["An Jux", "Bloodmoss, Sulfurous Ash"],
  "Protection": ["Uus Sanct", "Garlic, Ginseng, Sulfurous Ash"],
  "Strength": ["Uus Mani", "Mandrake Root, Nightshade"],
  "Bless": ["Rel Sanct", "Garlic, Mandrake Root"],
  "Fireball": ["Vas Flam", "Black Pearl"],
  "Magic Lock": ["An Por", "Garlic, Bloodmoss, Sulfurous Ash"],
  "Poison": ["In Nox", "Nightshade"],
  "Telekinesis": ["Ort Por Ylem", "Bloodmoss, Mandrake Root"],
  "Teleport": ["Rel Por", "Bloodmoss, Mandrake Root"],
  "Unlock": ["Ex Por", "Bloodmoss, Sulfurous Ash"],
  "Wall of Stone": ["In Sanct Ylem", "Bloodmoss, Garlic"],
  "Arch Cure": ["Vas An Nox", "Garlic, Ginseng, Mandrake Root"],
  "Arch Protection": ["Vas Uus Sanct", "Garlic, Ginseng, Mandrake Root, Sulfurous Ash"],
  "Curse": ["Des Sanct", "Nightshade, Garlic, Sulfurous Ash"],
  "Fire Field": ["In Flam Grav", "Black Pearl, Spider's Silk, Sulfurous Ash"],
  "Greater Heal": ["In Vas Mani", "Garlic, Ginseng, Mandrake Root, Spider's Silk"],
  "Lightning": ["Por Ort Grav", "Mandrake Root, Sulfurous Ash"],
  "Mana Drain": ["Ort Rel", "Black Pearl, Mandrake Root, Spider's Silk"],
  "Recall": ["Kal Ort Por", "Black Pearl, Bloodmoss, Mandrake Root"],
  "Blade Spirits": ["In Jux Hur Ylem", "Black Pearl, Mandrake Root, Nightshade"],
  "Dispel Field": ["An Grav", "Black Pearl, Spider's Silk, Sulfurous Ash, Garlic"],
  "Incognito": ["Kal In Ex", "Bloodmoss, Garlic, Nightshade"],
  "Magic Reflection": ["In Jux Sanct", "Garlic, Mandrake Root, Spider's Silk"],
  "Mind Blast": ["Por Corp Wis", "Black Pearl, Mandrake Root, Nightshade, Sulfurous Ash"],
  "Paralyze": ["An Ex Por", "Garlic, Mandrake Root, Spider's Silk"],
  "Poison Field": ["In Nox Grav", "Black Pearl, Nightshade, Spider's Silk"],
  "Summon Creature": ["Kal Xen", "Bloodmoss, Mandrake Root, Spider's Silk"],
  "Dispel": ["An Ort", "Garlic, Mandrake Root, Sulfurous Ash"],
  "Energy Bolt": ["Corp Por", "Black Pearl, Nightshade"],
  "Explosion": ["Vas Ort Flam", "Bloodmoss, Mandrake Root"],
  "Invisibility": ["An Lor Xen", "Bloodmoss, Nightshade"],
  "Mark": ["Kal Por Ylem", "Black Pearl, Bloodmoss, Mandrake Root"],
  "Mass Curse": ["Vas Des Sanct", "Garlic, Nightshade, Mandrake Root, Sulfurous Ash"],
  "Paralyze Field": ["In Ex Grav", "Black Pearl, Ginseng, Spider's Silk"],
  "Reveal": ["Wis Quas", "Bloodmoss, Sulfurous Ash"],
  "Chain Lightning": ["Vas Ort Grav", "Black Pearl, Bloodmoss, Mandrake Root, Sulfurous Ash"],
  "Energy Field": ["In Sanct Grav", "Black Pearl, Mandrake Root, Spider's Silk, Sulfurous Ash"],
  "Flamestrike": ["Kal Vas Flam", "Spider's Silk, Sulfurous Ash"],
  "Gate Travel": ["Vas Rel Por", "Black Pearl, Mandrake Root, Sulfurous Ash"],
  "Mana Vampire": ["Ort Sanct", "Black Pearl, Bloodmoss, Mandrake Root, Spider's Silk"],
  "Mass Dispel": ["Vas An Ort", "Garlic, Mandrake Root, Black Pearl, Sulfurous Ash"],
  "Meteor Swarm": ["Flam Kal Des Ylem", "Bloodmoss, Mandrake Root, Sulfurous Ash, Spider's Silk"],
  "Polymorph": ["Vas Ylem Rel", "Bloodmoss, Spider's Silk, Mandrake Root"],
  "Earthquake": ["In Vas Por", "Bloodmoss, Ginseng, Mandrake Root, Sulfurous Ash"],
  "Energy Vortex": ["Vas Corp Por", "Bloodmoss, Black Pearl, Mandrake Root, Nightshade"],
  "Resurrection": ["An Corp", "Bloodmoss, Garlic, Ginseng"],
  "Air Elemental": ["Kal Vas Xen Hur", "Bloodmoss, Mandrake Root, Spider's Silk"],
  "Summon Daemon": ["Kal Vas Xen Corp", "Bloodmoss, Mandrake Root, Spider's Silk, Sulfurous Ash"],
  "Earth Elemental": ["Kal Vas Xen Ylem", "Bloodmoss, Mandrake Root, Spider's Silk"],
  "Fire Elemental": ["Kal Vas Xen Flam", "Bloodmoss, Mandrake Root, Spider's Silk, Sulfurous Ash"],
  "Water Elemental": ["Kal Vas Xen An Flam", "Bloodmoss, Mandrake Root, Spider's Silk"],
};
// `graphic` = the school's spellbook ITEM id (world/equip/backpack graphic, as
// opposed to `book`'s GUMP art id above) — ServUO `Spellbook` subclass
// constructors. Used to match a known 0xBF/0x1B content entry (scene.spellbooks)
// to its school, and to find the player's own book of that school among their
// equip/backpack items (see `knownSpellbookFor`/`findOwnSpellbook`). Mastery has
// none: Skill Masteries aren't cast from a real spellbook's bit-mask content in
// the same way (ServUO `BookOfMasteries` uses its own gump), so that school
// always renders at full brightness regardless of `scene.spellbooks`.
const SPELL_SCHOOLS = [
  { key: "magery", label: "Magery", book: 0x08AC, graphic: 0x0EFA, iconStart: 0x08C0, spells: MAGERY_PAIRS },
  { key: "necromancy", label: "Necro", book: 0x2B00, graphic: 0x2253, iconStart: 0x5000, spells: NECROMANCY_SPELLS },
  { key: "chivalry", label: "Chivalry", book: 0x2B01, graphic: 0x2252, iconStart: 0x5100, spells: CHIVALRY_SPELLS },
  { key: "bushido", label: "Bushido", book: 0x2B07, graphic: 0x238C, iconStart: 0x5400, spells: BUSHIDO_SPELLS },
  { key: "ninjitsu", label: "Ninjitsu", book: 0x2B06, graphic: 0x23A0, iconStart: 0x5300, spells: NINJITSU_SPELLS },
  { key: "spellweaving", label: "Weaving", book: 0x2B2F, graphic: 0x2D50, iconStart: 0x59D8, spells: SPELLWEAVING_SPELLS },
  { key: "mysticism", label: "Mysticism", book: 0x2B32, graphic: 0x2D9D, iconStart: 0x5DC0, spells: MYSTICISM_SPELLS },
  { key: "mastery", label: "Mastery", book: 0x08AC, graphic: 0, iconStart: 0x0945, spells: MASTERY_SPELLS },
];
// T2A (The Second Age, pre-AOS) era: only Magery exists — Necromancy, Chivalry,
// Bushido, Ninjitsu, Spellweaving, Mysticism and Mastery are all later expansions.
// On a T2A shard hide every non-Magery school (no tab, not openable). Flip to false
// for a modern/AOS+ shard to expose all schools again. (scene.aos is NOT a reliable
// T2A signal — a T2A shard may still set Core.AOS server-side for OPL tooltips.)
const T2A = true;
const VISIBLE_SCHOOLS = T2A ? SPELL_SCHOOLS.filter((s) => s.key === "magery") : SPELL_SCHOOLS;
const SB_CORNER_L = 0x08BB;     // left page-turn corner gump (prev spread)
const SB_CORNER_R = 0x08BC;     // right page-turn corner gump (next spread)
// Two pages per spread, 8 spell rows per page (16 per spread); rows 44px apart,
// matching the book art. Left page icons at x=54, right page at x=221; first row
// y=48. Names sit to the right of each icon (flex row inside the entry).
const SB_PER_PAGE = 8, SB_ROW_H = 44, SB_ROW_Y0 = 48;
const SB_COL_X = [54, 221];
let spellbookOn = false;
let spellSchool = "magery";   // remembered across opens (module-scoped)
let spellPage = 0;            // current spread index (0-based)
// One overlaid spell entry: the spell's icon gump + its name. If the icon gump
// 404s (exotic schools), onerror collapses the <img> so only the name shows.
// Render the spell list (no book art): each spell shows its name, power words
// (mantra) and reagents — the classic in-fiction spellbook info. Magery is grouped
// by its 8 circles. Click a row to cast.
// Find the scene.spellbooks entry for `school` (matched by the book's ITEM
// graphic), or null if we don't know that school's content yet (book never
// opened this session, or `school.graphic` is 0 — Mastery). Callers must treat
// null as "unknown", NOT "empty book", and leave that school's rendering as
// it was before this feature existed (every spell at full brightness).
function knownSpellbookFor(school) {
  if (!school.graphic) return null;
  const list = (scene && scene.spellbooks) || [];
  return list.find((b) => ((b.graphic | 0) & 0xffff) === school.graphic) || null;
}
// Is global spell id `id` ABSENT from `book`'s 64-bit content mask? `content`
// arrives from anima-net split into two u32 halves, `lo` (bits 0..31) and `hi`
// (bits 32..63) — see `build_scene`'s doc for why (JS Number precision).
function spellMissing(book, id) {
  const bit = id - book.offset;
  if (bit < 0 || bit > 63) return true; // outside this book's range entirely
  const half = bit < 32 ? (book.lo >>> 0) : (book.hi >>> 0);
  return ((half >>> (bit % 32)) & 1) === 0;
}
// Find the player's own spellbook item of the given ITEM graphic: worn on the
// one-handed slot (Layer.OneHanded == 1) or sitting in any container we know
// the contents of (usually the backpack). null if we don't currently see one.
function findOwnSpellbook(graphic) {
  const p = scene && scene.player;
  if (p && p.equip) {
    const worn = p.equip.find((e) => (e.layer | 0) === 1 && ((e.g | 0) & 0xffff) === graphic);
    if (worn) return worn.serial >>> 0;
  }
  const items = (scene && scene.contItems) || [];
  const it = items.find((i) => ((i.g | 0) & 0xffff) === graphic);
  return it ? (it.serial >>> 0) : null;
}
// ServUO only ever sends 0xBF/0x1B spellbook content in reply to actually
// opening the book (Spellbook.OnDoubleClick → DisplayTo). So when the K window
// opens, double-click (via the existing use:<serial> plumbing) every VISIBLE
// school's book we haven't already asked about. The container dblclick handler
// already treats a spellbook specially (toggles this same window instead of
// opening a container view — see `isSpellbook`), and DisplayTo's other traffic
// (a repeat world/equip/container-slot packet, plus a 0x24 DisplaySpellbook we
// don't even parse) is otherwise a harmless no-op, so this has no visible side
// effect beyond the content arriving. Sent at most once per book serial ever,
// so reopening K repeatedly doesn't re-spam the request.
const spellbookContentRequested = new Set();
function requestUnknownSpellbookContents() {
  for (const school of VISIBLE_SCHOOLS) {
    if (!school.graphic || knownSpellbookFor(school)) continue; // Mastery, or already known
    const serial = findOwnSpellbook(school.graphic);
    if (serial == null || spellbookContentRequested.has(serial)) continue;
    spellbookContentRequested.add(serial);
    sendInput("use:" + serial);
  }
}
function renderSpellSchool() {
  const book = document.getElementById("sb-book");
  const school = VISIBLE_SCHOOLS.find((s) => s.key === spellSchool) || VISIBLE_SCHOOLS[0];
  const isMagery = school.key === "magery";
  const known = knownSpellbookFor(school); // null = content not known → don't dim anything
  let html = "", lastCircle = 0;
  school.spells.forEach(([id, name], idx) => {
    if (isMagery) {
      const circle = Math.floor(idx / 8) + 1;        // 8 spells per circle
      if (circle !== lastCircle) { html += `<div class="sp-circle">Circle ${circle}</div>`; lastCircle = circle; }
    }
    const info = isMagery ? MAGERY_INFO[name] : null;
    const iconId = school.iconStart + idx;           // k-th spell icon = iconStart + k
    // A spell the book doesn't actually contain is dimmed but still clickable —
    // there's no local rule enforcement, the server just refuses the cast.
    const missing = known != null && spellMissing(known, id);
    const title = missing ? `Cast ${name} (not in this book)` : `Cast ${name}`;
    // The icon is draggable out onto the screen → a floating quick-cast button.
    html += `<div class="sp-row${missing ? " sp-missing" : ""}" data-id="${id}" data-icon="${iconId}" data-name="${name}" title="${title}">`
      + `<img class="sp-icon" src="gump/${iconId}.png" alt="" draggable="true" crossorigin="anonymous"`
      + ` onerror="this.onerror=null;this.style.visibility='hidden'">`
      + `<div class="sp-txt"><div class="sp-name">${name}</div>`;
    if (info) {
      html += `<div class="sp-words">${info[0]}</div>`
        + `<div class="sp-reags">${info[1]}</div>`;
    }
    html += "</div></div>";
  });
  book.innerHTML = html;
  for (const t of document.querySelectorAll("#sb-tabs .sb-tab"))
    t.classList.toggle("sel", t.dataset.school === spellSchool);
  // Rebuilding the list dropped every `sp-active` class with the old nodes, so
  // clear the signature that would otherwise make the next refresh a no-op.
  sbActiveSig = null;
  refreshActiveSpells();
}
// Re-render the current school once new spellbook content arrives (scene.
// spellbooks changed) so the K window doesn't stay frozen at whatever it knew
// the moment it opened. Signature-gated: an unrelated scene poll (nothing
// spellbook-related changed) must not rebuild the list and reset scroll
// position for no reason.
let sbSpellbooksSig = null;
function refreshSpellbookContent() {
  if (!spellbookOn) return;
  const sig = JSON.stringify((scene && scene.spellbooks) || []);
  if (sig === sbSpellbooksSig) return;
  sbSpellbooksSig = sig;
  renderSpellSchool();
}
function buildSpellbook() {
  const tabs = document.getElementById("sb-tabs");
  if (tabs.childElementCount) { renderSpellSchool(); return; }   // wire once
  tabs.innerHTML = VISIBLE_SCHOOLS.map((s) =>
    `<div class="sb-tab" data-school="${s.key}">${s.label}</div>`).join("");
  tabs.addEventListener("click", (e) => {
    const tab = e.target.closest(".sb-tab");
    if (!tab) return;
    spellSchool = tab.dataset.school;
    renderSpellSchool();
  });
  document.getElementById("sb-book").addEventListener("click", (e) => {
    const row = e.target.closest(".sp-row");
    if (!row) return;
    // Cast → server replies with a target cursor when the spell needs one; the
    // existing target UI (scene.target) lets the player click the target.
    sendInput("cast:" + row.dataset.id);
  });
  wireSpellDragOut();   // dragging a spell icon out spawns a quick-cast button
  renderSpellSchool();
}
// Light up the spells the server says are currently running (`scene.activeSpells`,
// from 0xBF/0x25 ToggleSpecialAbility): Bushido/Ninjitsu stances and the like,
// which stay on until re-cast or broken. Toggled as a class on rows already in
// the DOM rather than by re-rendering, so it costs nothing per poll and cannot
// reset the book's scroll position (the reason `refreshSpellbookContent` is
// signature-gated). Unlike the ability bar this needs no optimistic echo: the
// server decides when a stance is on, and nothing here anticipates it.
let sbActiveSig = null;
function refreshActiveSpells() {
  const active = (scene && scene.activeSpells) || [];
  const sig = active.join(",");
  if (sig === sbActiveSig) return;
  sbActiveSig = sig;
  const on = new Set(active.map(Number));
  for (const row of document.querySelectorAll("#sb-book .sp-row"))
    row.classList.toggle("sp-active", on.has(Number(row.dataset.id)));
}
function refreshSpellMana() {
  const p = scene && scene.player;
  const el = document.getElementById("sb-mana");
  if (el) el.textContent = p ? `Mana: ${p.mana | 0} / ${p.manaMax | 0}` : "Mana: —";
}
function toggleSpellbook() {
  spellbookOn = !spellbookOn;
  const sb = document.getElementById("spellbook");
  sb.classList.toggle("on", spellbookOn);
  if (spellbookOn) { buildSpellbook(); refreshSpellMana(); requestUnknownSpellbookContents(); }
}
function closeSpellbook() {
  spellbookOn = false;
  document.getElementById("spellbook").classList.remove("on");
}

// --- skills window (0x3A, toggled by the 'L' key; ✕/Esc close) ---
// Lists every skill the server sent (scene.skills) with value/base/cap; the lock
// indicator cycles up(↑)/down(↓)/locked(🔒) on click → `skilllock:<id>:<next>`
// (0x3A SkillStatusChangeRequest). A ▸ affordance (or row double-click) invokes an
// active skill → `useskill:<id>` (0x12 ActionRequest type 0x24). Passive skills do
// nothing server-side, which is harmless. Names by id (0-based) from the standard
// UO skill table; unknown ids fall back to "Skill #id".
const SKILL_NAMES = [
  "Alchemy", "Anatomy", "Animal Lore", "Item ID", "Arms Lore", "Parrying", "Begging",
  "Blacksmithy", "Bowcraft/Fletching", "Peacemaking", "Camping", "Carpentry",
  "Cartography", "Cooking", "Detecting Hidden", "Discordance", "Evaluating Intelligence",
  "Healing", "Fishing", "Forensic Evaluation", "Herding", "Hiding", "Provocation",
  "Inscription", "Lockpicking", "Magery", "Resisting Spells", "Tactics", "Snooping",
  "Musicianship", "Poisoning", "Archery", "Spirit Speak", "Stealing", "Tailoring",
  "Animal Taming", "Taste Identification", "Tinkering", "Tracking", "Veterinary",
  "Swordsmanship", "Mace Fighting", "Fencing", "Wrestling", "Lumberjacking", "Mining",
  "Meditation", "Stealth", "Remove Trap", "Necromancy", "Focus", "Chivalry", "Bushido",
  "Ninjitsu", "Spellweaving", "Mysticism", "Imbuing", "Throwing",
];
function skillName(id) { return SKILL_NAMES[id] || ("Skill #" + id); }
// Active skills that do something on "use" (most via a target cursor). Other ids
// are still double-clickable but the ▸ button is hidden for them.
const USABLE_SKILLS = new Set([
  1, 2, 3, 4, 6, 9, 12, 14, 15, 16, 17, 19, 20, 21, 22, 23, 24, 28, 30, 32, 33, 35,
  36, 38, 46, 47, 48, 56,
]);
const LOCK_ICONS = ["↑", "↓", "🔒"]; // up ↑ / down ↓ / locked 🔒
const LOCK_TITLES = ["raise (click: lower)", "lower (click: lock)", "locked (click: raise)"];
let skillsOn = false;
function toggleSkills() {
  skillsOn = !skillsOn;
  const sk = document.getElementById("skills");
  sk.classList.toggle("on", skillsOn);
  if (skillsOn) { sk._sig = null; refreshSkills(); }
}

// ---- player status bar (UO's pull-out vitals/stats gump) ----
// A draggable window with the player's name, HP/Mana/Stam bars + numbers, and
// STR/DEX/INT/Gold. Toggle with the H key or by clicking the HUD name; its dragged
// position is remembered across sessions (localStorage), so you can "pull it out"
// and leave it where you like.
let statusOn = false;
function toggleStatus() {
  statusOn = !statusOn;
  const el = document.getElementById("statusbar");
  el.classList.toggle("on", statusOn);
  if (statusOn) { bringToFront(el); refreshStatus(scene); }
}
function closeStatus() {
  statusOn = false;
  document.getElementById("statusbar").classList.remove("on");
}
// HUD (top-right character status panel) + journal visibility toggles. Both persist.
// The journal lives inside the HUD, so hiding the HUD hides it too; the journal
// toggle hides just the log while the HUD stays. U = HUD, J = journal.
let hudHidden = false, journalHidden = false;
function applyHudVisibility() {
  const hud = document.getElementById("hud"); if (hud) hud.style.display = hudHidden ? "none" : "";
  const jr = document.getElementById("journal"); if (jr) jr.style.display = journalHidden ? "none" : "";
}
function loadHudVisibility() {
  hudHidden = localStorage.getItem("anima.hudHidden") === "1";
  journalHidden = localStorage.getItem("anima.journalHidden") === "1";
  applyHudVisibility();
}
function toggleHud() {
  hudHidden = !hudHidden;
  localStorage.setItem("anima.hudHidden", hudHidden ? "1" : "0");
  applyHudVisibility();
  setStatus(hudHidden ? "status panel hidden (U)" : "status panel shown");
}
function toggleJournal() {
  journalHidden = !journalHidden;
  localStorage.setItem("anima.journalHidden", journalHidden ? "1" : "0");
  applyHudVisibility();
}
function refreshStatus(s) {
  if (!statusOn || !s || !s.player) return;
  const p = s.player;
  set("st-name", p.name || "(unnamed)");
  set("st-hp-n", `${p.hits | 0} / ${p.hitsMax | 0}`); bar("st-hp", p.hits, p.hitsMax);
  set("st-mana-n", `${p.mana | 0} / ${p.manaMax | 0}`); bar("st-mana", p.mana, p.manaMax);
  set("st-stam-n", `${p.stam | 0} / ${p.stamMax | 0}`); bar("st-stam", p.stam, p.stamMax);
  set("st-str", p.str | 0); set("st-dex", p.dex | 0); set("st-int", p.int | 0);
  set("st-gold", p.gold | 0);
  // The rest of the sheet. `statsCap`/`tithing` are AOS-era fields a pre-AOS
  // shard simply leaves at 0 — shown as-is rather than hidden, so "0" reads as
  // "this shard doesn't use it" instead of the row vanishing unpredictably.
  set("st-weight", `${p.weight | 0} / ${p.weightMax | 0}`);
  set("st-statscap", p.statsCap | 0);
  set("st-followers", `${p.followers | 0} / ${p.followersMax | 0}`);
  set("st-damage", `${p.damageMin | 0} - ${p.damageMax | 0}`);
  set("st-luck", p.luck | 0);
  set("st-tithing", p.tithing | 0);
  set("st-armor", p.armor | 0);
  set("st-rfire", p.resistFire | 0);
  set("st-rcold", p.resistCold | 0);
  set("st-rpoison", p.resistPoison | 0);
  set("st-renergy", p.resistEnergy | 0);
}
function closeSkills() {
  skillsOn = false;
  document.getElementById("skills").classList.remove("on");
}
// Rebuild the list only when the skill data changes (value/base/cap/lock or set).
// ---- skill-gain / loss system messages ----
// UO never announced skill changes in T2A's silent way the renderer showed; the
// traditional client prints "Your skill in X has increased by 0.1." when a skill's
// BASE rises (ClassicUO diffs the 0x3A base value — `v` includes item/stat bonuses
// that fluctuate, so we track `b`). We append these as local journal lines.
const prevSkillBase = new Map();   // skill id -> last seen base (tenths)
let skillGainPrimed = false;       // skip the first scene so login isn't a flood
function checkSkillGains(s) {
  const skills = (s && s.skills) || [];
  if (!skillGainPrimed) {          // record baselines once; announce only later changes
    for (const sk of skills) prevSkillBase.set(sk.id | 0, sk.b | 0);
    skillGainPrimed = true;
    return;
  }
  for (const sk of skills) {
    const id = sk.id | 0, b = sk.b | 0;
    const prev = prevSkillBase.get(id);
    if (prev == null) { prevSkillBase.set(id, b); continue; }
    if (b !== prev) {
      const delta = (Math.abs(b - prev) / 10).toFixed(1);
      const verb = b > prev ? "increased" : "decreased";
      addSysMessage(`Your skill in ${skillName(id)} has ${verb} by ${delta}.`);
      prevSkillBase.set(id, b);
    }
  }
}

// Skills introduced in AOS or later (ServUO SkillName ≥ 46: Necromancy, Focus,
// Chivalry, Bushido, Ninjitsu, Spellweaving, Mysticism, Imbuing, Throwing). Everything
// below is the classic/T2A set. We only show T2A skills on a non-AOS shard; on AOS we
// list T2A and AOS skills in separate groups.
const AOS_SKILL_MIN = 46;
let skillSort = { key: "name", dir: 1 };  // key: name | value | base
function skillRowHtml(s) {
  const lock = ((s.lock | 0) % 3 + 3) % 3;
  const usable = USABLE_SKILLS.has(s.id | 0);
  return `<div class="sk-row${usable ? " usable" : ""}" data-id="${s.id}">`
    + `<span class="sk-lock" data-lock="${lock}" title="${LOCK_TITLES[lock]}">${LOCK_ICONS[lock]}</span>`
    + `<span class="sk-name" title="${skillName(s.id | 0)}">${skillName(s.id | 0)}</span>`
    + `<span class="sk-val">${((s.v | 0) / 10).toFixed(1)}</span>`
    + `<span class="sk-use" title="use skill">▸</span>`
    + (usable ? `<span class="sk-pop" title="pull out as a button">⧉</span>` : "")
    + `</div>`;
}
function refreshSkills() {
  if (!skillsOn) return;
  const win = document.getElementById("skills");
  const list = document.getElementById("sk-list");
  const aos = !!(scene && scene.aos);
  let skills = (scene && scene.skills) || [];
  if (!aos) skills = skills.filter((s) => (s.id | 0) < AOS_SKILL_MIN); // T2A only on non-AOS shards
  const sig = JSON.stringify({
    sort: skillSort, aos,
    s: skills.map((s) => `${s.id}:${s.v}:${s.b}:${s.c}:${s.lock}`),
  });
  if (win._sig === sig) return;
  win._sig = sig;
  // Total skill points = sum of base values (tenths → divide by 10).
  let totalBase = 0;
  for (const s of skills) totalBase += (s.b | 0);
  set("sk-total", `Total: ${(totalBase / 10).toFixed(1)}  ·  ${skills.length} skills`);
  // Sort header (clickable; same column toggles ascending/descending).
  const arrow = (k) => (skillSort.key === k ? (skillSort.dir > 0 ? " ▲" : " ▼") : "");
  document.getElementById("sk-sortbar").innerHTML = "Sort: "
    + `<span class="sk-sortk" data-k="name">Name${arrow("name")}</span>`
    + `<span class="sk-sortk" data-k="value">Value${arrow("value")}</span>`
    + `<span class="sk-sortk" data-k="base">Base${arrow("base")}</span>`;
  if (!skills.length) { list.innerHTML = '<div class="cont-empty">no skill data</div>'; return; }
  const cmp = (a, b) => {
    const d = skillSort.dir;
    if (skillSort.key === "name") return d * skillName(a.id | 0).localeCompare(skillName(b.id | 0));
    if (skillSort.key === "value") return d * ((a.v | 0) - (b.v | 0));
    return d * ((a.b | 0) - (b.b | 0)); // base
  };
  const rows = (arr) => arr.slice().sort(cmp).map(skillRowHtml).join("");
  if (aos) {
    // Group T2A vs AOS+ skills, each sorted.
    const t2a = skills.filter((s) => (s.id | 0) < AOS_SKILL_MIN);
    const aosk = skills.filter((s) => (s.id | 0) >= AOS_SKILL_MIN);
    let html = "";
    if (t2a.length) html += '<div class="sk-group">T2A</div>' + rows(t2a);
    if (aosk.length) html += '<div class="sk-group">AOS</div>' + rows(aosk);
    list.innerHTML = html;
  } else {
    list.innerHTML = rows(skills);
  }
}
// One delegated listener (wired once at startup): lock click cycles the lock; the
// ▸ button or a row double-click uses the skill.
function wireSkills() {
  const list = document.getElementById("sk-list");
  // Sort-header clicks: pick a column; clicking the active column flips direction.
  document.getElementById("sk-sortbar").addEventListener("click", (e) => {
    const k = e.target.closest && e.target.closest(".sk-sortk");
    if (!k) return;
    const key = k.dataset.k;
    skillSort = { key, dir: skillSort.key === key ? -skillSort.dir : 1 };
    const win = document.getElementById("skills"); if (win) win._sig = null;
    refreshSkills();
  });
  list.addEventListener("click", (e) => {
    const row = e.target.closest(".sk-row");
    if (!row) return;
    const id = row.dataset.id | 0;
    if (e.target.classList.contains("sk-lock")) {
      const next = ((e.target.dataset.lock | 0) + 1) % 3; // up→down→locked→up
      sendInput("skilllock:" + id + ":" + next);
      return;
    }
    if (e.target.classList.contains("sk-pop")) {
      addSkillButton(id);          // pull the skill out as a floating, draggable button
      return;
    }
    if (e.target.classList.contains("sk-use")) {
      sendInput("useskill:" + id);
    }
  });
  list.addEventListener("dblclick", (e) => {
    const row = e.target.closest(".sk-row");
    if (row) sendInput("useskill:" + (row.dataset.id | 0));
  });
}

// --- pulled-out skill buttons (UO SkillButtonGump): floating, draggable buttons
// that invoke a skill on click. Created from the skills list's ⧉ control, persisted
// in localStorage so they survive a reload. Click = use; drag = reposition; ✕ = remove.
const SKILLBTN_KEY = "anima.skillbtns";
let skillBtnCascade = 0;
function saveSkillButtons() {
  const arr = [];
  document.querySelectorAll(".skill-gump").forEach((el) => {
    arr.push({ id: +el.dataset.id | 0, x: parseInt(el.style.left, 10) || 0, y: parseInt(el.style.top, 10) || 0 });
  });
  try { localStorage.setItem(SKILLBTN_KEY, JSON.stringify(arr)); } catch (e) {}
}
function makeSkillButton(id, x, y) {
  id = id | 0;
  const el = document.createElement("div");
  el.className = "skill-gump";
  el.dataset.id = id;
  if (x == null) { x = 96 + (skillBtnCascade % 8) * 16; y = 130 + (skillBtnCascade % 8) * 16; skillBtnCascade++; }
  el.style.left = x + "px"; el.style.top = y + "px";
  el.innerHTML = `<span class="sg-name">${skillName(id)}</span><span class="sg-close gump-close">✕</span>`;
  // click vs drag: a stationary press uses the skill; a drag repositions it.
  el.addEventListener("mousedown", (e) => {
    if (e.target.classList.contains("sg-close")) return;
    e.preventDefault();
    bringToFront(el);
    const r = el.getBoundingClientRect();
    const ox = e.clientX - r.left, oy = e.clientY - r.top;
    const dx0 = e.clientX, dy0 = e.clientY;
    let moved = false;
    const move = (ev) => {
      if (Math.abs(ev.clientX - dx0) > 3 || Math.abs(ev.clientY - dy0) > 3) moved = true;
      el.style.left = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - ox)) + "px";
      el.style.top = Math.max(0, Math.min(window.innerHeight - 20, ev.clientY - oy)) + "px";
    };
    const up = () => {
      window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
      if (moved) { saveSkillButtons(); }
      else { sendInput("useskill:" + id); el.classList.add("flash"); setTimeout(() => el.classList.remove("flash"), 160); }
    };
    window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
  });
  el.querySelector(".sg-close").addEventListener("click", () => { el.remove(); saveSkillButtons(); });
  document.body.appendChild(el);
  return el;
}
function addSkillButton(id) { makeSkillButton(id, null, null); saveSkillButtons(); }

// "Show all names" (G key — ClassicUO's Ctrl+Shift all-names): single-click every
// in-view character — self, players, NPCs and animals — so the server returns each
// name. The names arrive as overhead text (shown regardless of the name-label
// setting) and also fill the persistent labels. Capped so it never floods the link.
function requestAllNames() {
  if (!scene) return;
  let n = 0;
  if (scene.player) { sendInput("click:" + (scene.player.serial >>> 0)); n++; }
  for (const m of scene.mobiles || []) {
    if (n >= 60) break;
    sendInput("click:" + (m.serial >>> 0));
    n++;
  }
  setStatus(`querying ${n} name${n === 1 ? "" : "s"}…`);
}
function loadSkillButtons() {
  let arr = [];
  try { arr = JSON.parse(localStorage.getItem(SKILLBTN_KEY) || "[]"); } catch (e) { arr = []; }
  for (const b of arr) makeSkillButton(b.id, b.x, b.y);
}

// --- spell quick-cast buttons: drag a spell icon out of the spellbook onto the
// screen → a floating icon button that casts on click, drags to reposition, ✕ to
// remove. Persisted (like skill buttons). ---
const SPELLBTN_KEY = "anima.spellbtns";
let spellBtnCascade = 0;
let spellDrag = null;   // { id, icon, name } while dragging an icon out of the book
function saveSpellButtons() {
  const arr = [];
  document.querySelectorAll(".spell-gump").forEach((el) => {
    arr.push({ id: +el.dataset.id | 0, icon: +el.dataset.icon | 0, name: el.dataset.name || "",
      x: parseInt(el.style.left, 10) || 0, y: parseInt(el.style.top, 10) || 0 });
  });
  try { localStorage.setItem(SPELLBTN_KEY, JSON.stringify(arr)); } catch (e) {}
}
function makeSpellButton(id, icon, name, x, y) {
  const el = document.createElement("div");
  el.className = "spell-gump";
  el.dataset.id = id | 0; el.dataset.icon = icon | 0; el.dataset.name = name || "";
  if (x == null) { x = 120 + (spellBtnCascade % 8) * 16; y = 150 + (spellBtnCascade % 8) * 16; spellBtnCascade++; }
  el.style.left = x + "px"; el.style.top = y + "px";
  el.title = name ? ("Cast " + name) : "Cast";
  el.innerHTML = `<img class="spell-gump-ic" src="gump/${icon | 0}.png" alt="" crossorigin="anonymous"`
    + ` onerror="this.style.visibility='hidden'"><span class="sg-close gump-close">✕</span>`;
  // Stationary press = cast; a drag repositions it (same model as skill buttons).
  el.addEventListener("mousedown", (e) => {
    if (e.target.classList.contains("sg-close")) return;
    e.preventDefault(); bringToFront(el);
    const r = el.getBoundingClientRect();
    const ox = e.clientX - r.left, oy = e.clientY - r.top, dx0 = e.clientX, dy0 = e.clientY;
    let moved = false;
    const move = (ev) => {
      if (Math.abs(ev.clientX - dx0) > 3 || Math.abs(ev.clientY - dy0) > 3) moved = true;
      el.style.left = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - ox)) + "px";
      el.style.top = Math.max(0, Math.min(window.innerHeight - 20, ev.clientY - oy)) + "px";
    };
    const up = () => {
      window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
      if (moved) saveSpellButtons();
      else { sendInput("cast:" + (id | 0)); el.classList.add("flash"); setTimeout(() => el.classList.remove("flash"), 160); }
    };
    window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
  });
  el.querySelector(".sg-close").addEventListener("click", () => { el.remove(); saveSpellButtons(); });
  document.body.appendChild(el);
  return el;
}
function loadSpellButtons() {
  let arr = [];
  try { arr = JSON.parse(localStorage.getItem(SPELLBTN_KEY) || "[]"); } catch (e) { arr = []; }
  for (const b of arr) makeSpellButton(b.id, b.icon, b.name, b.x, b.y);
}
// Wire the HTML5 drag-out: dragging a `.sp-icon` from the book drops a button on the
// screen. Registered once (idempotent via a flag).
let spellDragWired = false;
function wireSpellDragOut() {
  if (spellDragWired) return; spellDragWired = true;
  document.getElementById("sb-book").addEventListener("dragstart", (e) => {
    const ic = e.target.closest && e.target.closest(".sp-icon");
    const row = ic && ic.closest(".sp-row");
    if (!row) return;
    spellDrag = { id: +row.dataset.id | 0, icon: +row.dataset.icon | 0, name: row.dataset.name || "" };
    e.dataTransfer.effectAllowed = "copy";
    try { e.dataTransfer.setData("text/plain", String(spellDrag.id)); } catch (_) {}
  });
  document.addEventListener("dragover", (e) => { if (spellDrag) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } });
  document.addEventListener("drop", (e) => {
    if (!spellDrag) return;
    e.preventDefault();
    makeSpellButton(spellDrag.id, spellDrag.icon, spellDrag.name, e.clientX - 22, e.clientY - 22);
    saveSpellButtons();
    spellDrag = null;
  });
  document.addEventListener("dragend", () => { spellDrag = null; });
}
// --- party panel (0xBF/0x06, toggled by the 'Y' key; ✕/Esc close) ---
// Lists party members with a name + health bar (hits/hitsMax), the leader marked
// with a crown. "Invite" sends `partyinvite` (the server then opens a target
// cursor — the existing target UI handles the click); "Leave" sends `partyleave`.
// When `scene.party.invite` is non-zero someone invited us: an Accept/Decline
// prompt appears (and the panel auto-opens so it's visible). Member name/hits are
// only known while that member is in view; out-of-view members show "Member"/no bar.
let partyOn = false;
function toggleParty() {
  partyOn = !partyOn;
  const w = document.getElementById("party");
  w.classList.toggle("on", partyOn);
  if (partyOn) { w._sig = null; refreshParty(); }
}
function closeParty() {
  partyOn = false;
  document.getElementById("party").classList.remove("on");
}
// Rebuild only when the party data changes (members, hits, leader, or invite).
function refreshParty() {
  const party = (scene && scene.party) || { leader: 0, members: [], invite: 0 };
  const invite = party.invite | 0;
  // Auto-open the panel when an invite arrives so the prompt is never missed.
  if (invite && !partyOn) {
    partyOn = true;
    document.getElementById("party").classList.add("on");
  }
  if (!partyOn) return;
  const win = document.getElementById("party");
  const sig = `${party.leader}|${invite}|` +
    (party.members || []).map((m) => `${m.serial}:${m.hits}:${m.hitsMax}:${m.name}`).join(",");
  if (win._sig === sig) return;
  win._sig = sig;

  // Incoming-invite prompt (Accept / Decline).
  const prompt = document.getElementById("pt-invite-prompt");
  prompt.classList.toggle("on", !!invite);
  if (invite) {
    prompt.innerHTML =
      '<div class="pt-itext">A party invitation is pending.</div>' +
      '<div class="pt-irow">' +
      `<button class="pt-btn" data-act="partyaccept" data-leader="${invite}">Accept</button>` +
      `<button class="pt-btn" data-act="partydecline" data-leader="${invite}">Decline</button>` +
      '</div>';
  } else {
    prompt.innerHTML = "";
  }

  // Member list with health bars.
  const list = document.getElementById("pt-list");
  const members = party.members || [];
  if (!members.length) {
    list.innerHTML = '<div class="pt-empty">Not in a party.</div>';
  } else {
    let html = "";
    for (const m of members) {
      const isLeader = (m.serial | 0) === (party.leader | 0);
      const max = m.hitsMax | 0;
      const pct = max > 0 ? Math.max(0, Math.min(100, Math.round((m.hits | 0) * 100 / max))) : 0;
      // Another player's HP is always NORMALIZED by the server (ServUO
      // AttributeNormalizer, max 25) — nobody sees a stranger's real hit points in
      // UO — so a full-health ally arrives as "25/25", which is meaningless to
      // print. Show a percentage for other members; only our OWN entry carries the
      // real hits/max (our unnormalized self status), so show true numbers there.
      const isSelf = (m.serial | 0) === ((scene.player && scene.player.serial) | 0);
      const hp = max <= 0 ? "—" : isSelf ? `${m.hits | 0}/${max}` : `${pct}%`;
      const name = (m.name || "Member").replace(/[<>&]/g, "");
      html += `<div class="pt-row${isLeader ? " leader" : ""}">`
        + `<div class="pt-head">`
        + (isLeader ? '<span class="pt-crown" title="leader">♛</span>' : "")
        + `<span class="pt-name">${name}</span>`
        + `<span class="pt-hp">${hp}</span>`
        + `</div>`
        + `<div class="pt-bar"><i style="width:${pct}%"></i></div>`
        + `</div>`;
    }
    list.innerHTML = html;
  }
}
// Wire the party panel once at startup: Invite/Leave buttons + the Accept/Decline
// prompt (delegated so it survives innerHTML rebuilds).
function wireParty() {
  document.getElementById("pt-invite").addEventListener("click", () => sendInput("partyinvite"));
  document.getElementById("pt-leave").addEventListener("click", () => sendInput("partyleave"));
  document.getElementById("pt-invite-prompt").addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-act]");
    if (!btn) return;
    sendInput(btn.dataset.act + ":" + (btn.dataset.leader | 0));
  });
}
// --- secure trade windows (0x6F player-to-player trade, one per session) ---
// scene.trades = [{ opponent, opponentSerial, myCont, theirCont, myAccept,
// theirAccept, myOfferGold, myOfferPlat, theirOfferGold, theirOfferPlat,
// balanceGold, balancePlat }, …] (see anima-net scene.rs trades_json). Items on
// each side are ordinary scene.contItems keyed by container serial —
// SecureTradeEquip reuses the 0x25 AddToContainer wire format server-side, so
// filtering by myCont/theirCont is all a container window would do anyway.
// Unlike the party panel (toggled by 'Y') or the backpack (toggled by 'I'), a
// trade is something the SERVER opens — trading is peer-to-peer with no
// consent required, so more than one stranger can have a session open with us
// at once. One window per session, keyed by OUR OWN container serial
// (`myCont`, the value every outgoing trade command addresses), built/torn
// down the same way the container/gump dialog families manage their multi-window
// lifecycle: build on first sight, refresh in place while the signature is
// unchanged, remove once the session drops off scene.trades.
const tradeCascade = { n: 0, left: 340, top: 90, wrap: 6, step: 24 };
// Build one side's item grid from scene.contItems, reusing the exact `.cont-item`
// markup/styling a normal container window uses. `readOnly` (the opponent's
// side) skips the drag-arm data attribute so `setupItemDnD` won't let us lift
// items we don't own; both sides still show the hover OPL tooltip (delegated
// on `.cont-item[data-serial]` regardless of the `ro` flag).
function renderTradeGrid(gridEl, cont, readOnly) {
  const items = (scene && scene.contItems || []).filter((it) => (it.cont >>> 0) === (cont >>> 0));
  gridEl.innerHTML = "";
  if (!items.length) { gridEl.innerHTML = '<div class="cont-empty">(empty)</div>'; return; }
  for (const it of items) {
    const cell = document.createElement("div");
    cell.className = "cont-item";
    cell.title = readOnly ? "" : "drag to move";
    cell.draggable = false;
    cell.dataset.serial = it.serial >>> 0;
    cell.dataset.g = it.g;
    cell.dataset.amount = (it.amount | 0) || 1;
    cell.dataset.st = it.st ? "1" : "0";
    cell.dataset.hue = it.hue | 0;          // carried into the drag ghost on lift
    if (readOnly) cell.dataset.ro = "1";
    const img = document.createElement("img");
    img.className = "cont-icon";
    img.src = `art/static/${stackGraphic(it.g, it.amount | 0)}.png${hueQuery(it.hue)}`;
    img.draggable = false;
    img.onerror = () => { img.style.visibility = "hidden"; };
    cell.appendChild(img);
    if ((it.amount | 0) > 1) {
      const a = document.createElement("span");
      a.className = "cont-amt"; a.textContent = it.amount;
      cell.appendChild(a);
    }
    gridEl.appendChild(cell);
  }
}
function tradeInputFocused(win) {
  const a = document.activeElement;
  return a === win.goldIn || a === win.platIn;
}
// Send our gold/plat offer, clamped client-side to the account balance the
// server last gave us (action-4 UpdateLedger, `win.balanceGold`/`balancePlat`)
// — mirrors ClassicUO's TradingGump entry handler, which clamps rather than
// letting the player type more than they have.
function sendTradeGold(win) {
  const gold = Math.max(0, Math.min(win.balanceGold, parseInt(win.goldIn.value, 10) || 0));
  const plat = Math.max(0, Math.min(win.balancePlat, parseInt(win.platIn.value, 10) || 0));
  sendInput("tradegold:" + win.myCont + ":" + gold + ":" + plat);
}
function cancelTrade(myCont) {
  sendInput("tradecancel:" + myCont);
  dismissDialog("trades", myCont); // close locally now — don't wait a poll for the server's echo
}
function buildTradeWindow(myCont) {
  const el = document.createElement("div");
  el.className = "gump-win trade-win";
  const off = (tradeCascade.n++ % tradeCascade.wrap) * tradeCascade.step;
  el.style.left = (tradeCascade.left + off) + "px";
  el.style.top = (tradeCascade.top + off) + "px";
  el.innerHTML =
    '<div class="gump-title"><span>TRADE · <span class="tr-name"></span></span><span class="gump-close">✕</span></div>'
    + '<div class="gump-body">'
    + '<div class="tr-cols">'
    + '<div class="tr-col">'
    + '<div class="tr-col-title">You</div>'
    + '<div class="tr-grid tr-mine-grid"></div>'
    + '<label class="tr-accept"><input type="checkbox" class="tr-accept-cb"> I accept</label>'
    + '<div class="tr-gold-row">'
    + '<input class="tr-gold-in tr-gold" type="number" min="0" inputmode="numeric" placeholder="gold" autocomplete="off">'
    + '<input class="tr-gold-in tr-plat" type="number" min="0" inputmode="numeric" placeholder="plat" autocomplete="off">'
    + '</div>'
    + '<div class="tr-balance"></div>'
    + '</div>'
    + '<div class="tr-col">'
    + '<div class="tr-col-title tr-their-name">Them</div>'
    + '<div class="tr-grid tr-theirs-grid"></div>'
    + '<span class="tr-accept tr-their-accept">waiting…</span>'
    + '<div class="tr-their-gold">0 gold / 0 plat</div>'
    + '</div>'
    + '</div>'
    + '<button class="dlg-btn tr-cancel">Cancel Trade</button>'
    + '</div>';
  document.body.appendChild(el);
  const win = {
    el, sig: null, myCont,
    goldIn: el.querySelector(".tr-gold"), platIn: el.querySelector(".tr-plat"),
    balanceGold: 0, balancePlat: 0,
  };
  el.querySelector(".gump-close").addEventListener("click", () => cancelTrade(myCont));
  const cancelBtn = el.querySelector(".tr-cancel");
  cancelBtn.addEventListener("click", () => cancelTrade(myCont));
  el.querySelector(".tr-accept-cb").addEventListener("change", (e) => {
    sendInput("tradeaccept:" + myCont + ":" + (e.target.checked ? "1" : "0"));
    // A checkbox is an <input>, so isTypingTarget() treats it as a typing target
    // while it holds focus — EVERY game key (not just letters) would silently
    // die, and a stray Space would natively re-toggle it. Blur to release focus,
    // matching how other windows (e.g. closeChat) avoid stealing the keyboard.
    e.target.blur();
  });
  for (const inp of [win.goldIn, win.platIn]) {
    inp.addEventListener("change", () => sendTradeGold(win));
    // Keep Enter/Esc local to this field (same pattern as the split/prompt
    // dialogs) so typing a gold amount never leaks a digit/movement key to
    // the global game-input handler.
    inp.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.code === "Enter" || e.code === "NumpadEnter") { e.preventDefault(); sendTradeGold(win); inp.blur(); }
    });
  }
  makeDraggable(el, el.querySelector(".gump-title"));
  return win;
}
// Rebuild a session's window only when its data (or either side's items)
// actually changed.
// What "this trade changed" means: both sides' acceptance/offers plus every item
// in either container. The goods live in scene.contItems, not on the trade item
// itself, which is why this needs the snapshot as well as the session.
function tradeSignature(t, scene) {
  const myCont = t.myCont >>> 0, theirCont = t.theirCont >>> 0;
  const items = ((scene && scene.contItems) || []).filter(
    (it) => (it.cont >>> 0) === myCont || (it.cont >>> 0) === theirCont
  );
  return [
    t.theirCont, t.opponent, t.myAccept, t.theirAccept,
    t.myOfferGold, t.myOfferPlat, t.theirOfferGold, t.theirOfferPlat,
    t.balanceGold, t.balancePlat,
    items.map((it) => `${it.cont >>> 0}:${it.serial >>> 0}:${it.g}:${it.amount | 0}:${it.hue | 0}`).join(","),
  ].join("|");
}
function renderTradeWindow(win, t) {
  win.el.querySelector(".tr-name").textContent = t.opponent || "someone";
  win.el.querySelector(".tr-their-name").textContent = t.opponent || "Them";
  renderTradeGrid(win.el.querySelector(".tr-mine-grid"), t.myCont, false);
  renderTradeGrid(win.el.querySelector(".tr-theirs-grid"), t.theirCont, true);
  win.el.querySelector(".tr-accept-cb").checked = !!t.myAccept;
  const theirAccept = win.el.querySelector(".tr-their-accept");
  theirAccept.textContent = t.theirAccept ? "✓ accepted" : "waiting…";
  theirAccept.classList.toggle("yes", !!t.theirAccept);
  win.el.querySelector(".tr-their-gold").textContent = `${t.theirOfferGold | 0} gold / ${t.theirOfferPlat | 0} plat`;
  win.balanceGold = t.balanceGold | 0;
  win.balancePlat = t.balancePlat | 0;
  win.el.querySelector(".tr-balance").textContent = `balance: ${win.balanceGold} gold / ${win.balancePlat} plat`;
  // Cap what can be typed to the account balance the server last gave us —
  // mirrors ClassicUO's TradingGump clamping the entry to `Gold`/`Platinum`.
  win.goldIn.max = win.balanceGold;
  win.platIn.max = win.balancePlat;
  // Don't clobber the field while the player is mid-keystroke in it.
  if (!tradeInputFocused(win)) {
    win.goldIn.value = t.myOfferGold | 0;
    win.platIn.value = t.myOfferPlat | 0;
  }
}
// Auto-open a window for each session in scene.trades, refresh the ones whose
// data changed, and auto-close any window whose session dropped off the list
// (cancelled, completed, or the opponent walked away).
registerDialog({
  id: "trades",
  source: (scene) => (scene && scene.trades) || [],
  key: (t) => t.myCont >>> 0,
  sig: tradeSignature,
  // Suppressed by SESSION, not content: a cancel is terminal, but the trade's
  // signature keeps changing (items move, gold is offered) right up until the
  // server tears the session down — a content-keyed guard would let one of those
  // updates reopen the window the player just closed.
  dismiss: "session",
  build: (t, { key }) => buildTradeWindow(key),
  update: (win, t) => renderTradeWindow(win, t),
});

// ---- treasure/decoration map windows (0x90/0xF5 DisplayMap(New) + 0x56
// MapCommand — ServUO `Scripts/Items/Tools/MapItem.cs`; one window per
// serial, built dynamically like .trade-win/.container-win) ----
const mapCascade = { n: 0, left: 260, top: 80 };
function closeMapWindow(serial) {
  // No dismiss guard: open:"seq" already means a lingering snapshot can't reopen
  // this window — only a fresh 0x90/0xF5 (a higher openSeq) can.
  closeDialog("maps", serial);
}
// Pin editing (0x56). ServUO gates EVERY mutator on the map being in edit
// mode (`ValidateEdit` = `m_Editable && Validate(from)`, and `m_Editable`
// starts false), so the Edit button is not a local view state — it sends
// command 6 and the server answers with its own command 7 carrying the
// verdict, which can be "still not editable" for a map that is out of reach,
// protected, or someone else's. That is why `editable` is read from the scene
// rather than tracked here.
function buildMapWindow(serial) {
  const { el, body } = makeWindowFrame({
    cls: "map-win", title: "Map", cascade: mapCascade,
    onClose: () => closeMapWindow(serial),
  });
  body.innerHTML = '<div class="map-tools">'
    + '<button class="map-edit">Edit</button>'
    + '<button class="map-clear">Clear pins</button>'
    + '<span class="map-hint"></span></div>'
    + '<div class="map-canvas"></div>';
  const canvas = el.querySelector(".map-canvas");
  el.querySelector(".map-edit").addEventListener("click", () => sendInput("mapedit:" + serial));
  el.querySelector(".map-clear").addEventListener("click", () => sendInput("mappinclr:" + serial));
  // Click empty parchment to drop a pin; click a pin to remove it. Coordinates
  // are the map's own pixel space, which is exactly what the canvas is sized
  // to — see `renderMapWindow` for why no rescale is involved.
  canvas.addEventListener("click", (e) => {
    if (!canvas._editable) return;
    const pin = e.target.closest(".map-pin");
    if (pin) {
      const i = pin.dataset.index | 0;
      // Index 0 is the chest on a decoded treasure map and ServUO's
      // `RemovePin` refuses it; say so instead of sending a silent no-op.
      if (i === 0) { addSysMessage("The treasure pin cannot be removed."); return; }
      sendInput(`mappindel:${serial}:${i}`);
      return;
    }
    const r = canvas.getBoundingClientRect();
    const x = Math.round(e.clientX - r.left), y = Math.round(e.clientY - r.top);
    sendInput(`mappin:${serial}:${x}:${y}`);
  });
  return { el, canvas };
}
// Rebuild a map window's art/pins only when its content signature changed.
// Bounds/size never change for a given serial in practice, but PINS do via
// bare 0x56 traffic that does NOT bump `openSeq` (see `MapView::open_seq`'s
// doc) — so this must run on every poll for an already-open window,
// independent of `refreshMapWindows`'s open-a-NEW-window gate. The
// background is the (constant, ServUO always sends 0x139D) parchment gump
// art stretched via CSS to the map's own w×h box — pins are drawn at their
// raw wire (x, y) with NO further rescale, because stretching the background
// to fill that exact box already makes it line up (see `MapView`'s Rust doc
// for why no client-side pin math is needed either way). Pin index 0 is the
// treasure/chest pin (ServUO `MapItem.RemovePin` refuses to remove it) —
// drawn with the `.chest` variant so it reads as the goal.
function mapSignature(m) {
  return JSON.stringify([m.gumpArt, m.w, m.h, m.pins, m.editable]);
}
function renderMapWindow(win, m) {
  const w = m.w | 0, h = m.h | 0;
  const c = win.canvas;
  c.style.width = w + "px";
  c.style.height = h + "px";
  c._editable = !!m.editable;
  c.classList.toggle("editing", !!m.editable);
  const tools = win.el.querySelector(".map-tools");
  if (tools) {
    tools.querySelector(".map-edit").classList.toggle("on", !!m.editable);
    tools.querySelector(".map-hint").textContent = m.editable
      ? "click to add a pin · click a pin to remove"
      : "";
  }
  c.innerHTML = `<img class="map-bg" src="gump/${m.gumpArt | 0}.png" alt=""`
    + ` onerror="this.onerror=null;this.style.display='none'">`;
  (m.pins || []).forEach((p, i) => {
    const pin = document.createElement("div");
    pin.className = "map-pin" + (i === 0 ? " chest" : "");
    pin.dataset.index = i;
    pin.style.left = (p[0] | 0) + "px";
    pin.style.top = (p[1] | 0) + "px";
    pin.title = i === 0 ? "treasure" : ("pin " + i);
    c.appendChild(pin);
  });
}
// Open a NEW window only when a serial's `openSeq` (scene.maps[].openSeq)
// advances past what we've already opened for (the open:"seq" policy):
// this is what stops a user-closed map window from popping back open on
// every poll just because World still carries the same MapView. Content of
// any ALREADY-open window is still refreshed every poll regardless (a pin
// can change via a bare 0x56 that doesn't bump `openSeq`). A window whose
// map fell out of scene.maps entirely (the item was deleted, or a facet
// switch purged it — see `World::on_map_change`) is closed to match.
registerDialog({
  id: "maps",
  source: (scene) => (scene && scene.maps) || [],
  key: (m) => m.serial >>> 0,
  sig: mapSignature,
  // The server re-sends the same map item on every content update, so presence
  // in the snapshot can't mean "open me" — only a fresh 0x90/0xF5, which bumps
  // openSeq, does. That's also what lets a closed map stay closed without a
  // dismiss guard.
  open: "seq",
  seq: (m) => m.openSeq | 0,
  build: (m, { key }) => buildMapWindow(key),
  update: renderMapWindow,
  reopen: (win) => bringToFront(win.el),
});

