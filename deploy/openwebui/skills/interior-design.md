---
name: Interior Design
description: Space planning, lighting, colour and style knowledge, plus how to write a render instruction that restyles a room photo without changing the room.
---

# Interior design

## Reading a room photo

Before proposing anything, note what you can see and what cannot change:

- **Fixed:** walls, windows, doors, radiators, ceiling height, floor plan, built-ins. A render must
  keep these exactly; a proposal that moves them is a renovation, and says so.
- **Light:** which way the windows face (if guessable), how much daylight, what artificial light
  exists. Most rooms have one ceiling light and nothing else.
- **Materials and colours:** floor, walls, the large furniture, textiles.
- **Scale:** estimate room size from doors (~80 cm wide, ~2 m tall) and furniture. Say when an
  estimate is rough, and ask for measurements when a proposal depends on them.

## Space planning

- Walkways: 90–120 cm for main routes, 60–90 cm between furniture, 45 cm only where nobody passes
  often. Plan routes first, then place furniture around them.
- Sofa to coffee table: 40–50 cm. Coffee table about two thirds the length of the sofa.
- Dining chair pull-back: 90 cm from table edge to wall. Bed: 60 cm free on each used side.
- Rugs: at least the front legs of every seat in the group stand on the rug. A rug that floats in
  the middle makes a room look smaller.
- Anchor each zone (sitting, reading, dining) with a rug, a light, or both.
- TV: viewing distance about 1.5–2.5× the screen diagonal; centre of screen near seated eye height.

## Lighting — always in layers

1. **Ambient** — general light: ceiling, wall washers, indirect.
2. **Task** — reading lamps, desk, kitchen counters: 2–3× brighter where the work happens.
3. **Accent** — picture lights, shelf strips, a lamp in a dark corner.

Rough levels: living room 150–300 lux, reading and desk 300–500, kitchen counters 300–750,
bedroom 50–150. Warm white (2700–3000 K) for living spaces in the evening; 4000 K for work and
kitchens. A room lit only from the ceiling is the most common fix to suggest.

## Colour

- **60 / 30 / 10:** a dominant colour (walls, large surfaces), a secondary (furniture, textiles),
  an accent (cushions, art, one object).
- North-facing rooms get cool light: warm the palette. South-facing rooms tolerate cool colours.
- Paint looks different in every room and at every hour (metamerism): always tell people to test
  a sample on two walls and look at it morning and evening before buying.
- Name colours concretely ("warm off-white with a hint of yellow", "deep olive", "terracotta") and
  give 3–5 of them as a palette.

## Styles, in one line each

- **Scandinavian:** light wood, white and grey, wool and linen, maximise daylight, cosy not bare.
- **Japandi:** low furniture, earth tones, natural textures, empty space treated as a feature.
- **Mid-century modern:** tapered legs, walnut, organic curves, mustard, teal and olive accents.
- **Industrial:** exposed brick or concrete, black metal, leather, warehouse light fittings.
- **Modern classic:** mouldings, symmetry, muted colour, a few antique-style pieces in a clean room.
- **Mediterranean:** lime-wash walls, terracotta, rattan, blue or green accents, strong daylight.
- **Maximalist:** layered pattern and colour held together by a repeated palette.

## Budget tiers

When proposing changes, sort them so people can stop at any step:

1. **Free to cheap:** rearrange, declutter, move lamps, swap cushions and throws.
2. **Moderate:** paint, rug, curtains, lamps, plants, one statement piece.
3. **Bigger:** sofa, flooring, built-in storage, new light points (needs an electrician).

## Writing a render instruction

The image model sees the photo and your instruction, nothing else: the instruction must stand
alone. Structure it as:

1. **What stays:** "Keep the room geometry, walls, window and door positions, ceiling and camera
   angle exactly as in the photo."
2. **Each change, explicitly:** name the object, then its new form, material and colour — "replace
   the brown leather sofa with a low linen sofa in warm off-white". Unnamed objects may vanish or
   be invented, so mention the ones that must stay too ("keep the radiator under the window").
3. **Light and mood:** time of day, which lamps are on, warmth of the light.
4. **Finish:** "Photorealistic interior photograph, natural colours, no text or watermark."

For a follow-up ("make the sofa green"), edit the latest render and change only what was asked:
"Keep everything in this image the same except …".

Without a photo, describe a complete room instead: type and size, window positions, style,
palette, the main pieces, lighting, camera position ("eye level from the doorway, wide angle").

## Limits — say these plainly

- Walls, structure, gas, electrics and plumbing need a qualified professional; never suggest
  removing a wall without saying it may be load-bearing.
- A render is a mood picture, not a measured plan: real furniture must be checked against
  measurements before buying.
