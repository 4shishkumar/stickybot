# Sounds built into the firmware

Every clip in `src/sounds_clips.cpp`, where it comes from and under what licence. All are
public domain (CC0) or made from scratch for this project. `tools/make_sounds.py` builds them
(mono, 16 kHz, bass under 400 Hz removed, compressed to be loud, IMA-ADPCM); `make sounds`
re-runs it.

## Kenney sound packs (CC0 1.0)

By Kenney ([kenney.nl](https://kenney.nl)), released under
[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/). The packs are downloaded by
`make_sounds.py` into `tools/sounds/` (not committed).

| Clip | Pack | File(s) |
|---|---|---|
| `drop_002`, `select_003`, `confirmation_001`, `confirmation_002`, `back_002`, `error_004`, `error_008`, `open_002`, `close_003`, `pluck_001`, `question_001`, `glass_002` | Interface Sounds | the same names, `.ogg` |
| `unlock_click` | Interface Sounds | `toggle_002` + `glass_001` |
| `jingle_win` | Music Jingles | `jingles_NES00` |
| `jingle_sax` | Music Jingles | `jingles_SAX07` |

## Synthesized for StickyBot (firmware 2.7.0)

Made from scratch by `tools/synth_sounds.py` (numpy only: filtered noise, oscillators and
envelopes, from fixed random seeds, so a rebuild gives the same bytes). No recordings or
samples are used, so there is no third-party licence; they're part of this project (MIT, see
[LICENSE](../LICENSE)).

| Clip | Length | Flash | How it's made |
|---|---|---|---|
| `applause` | 1.80 s | 14,940 B | ~25 "people" clapping at ~4.5 claps/s with jitter; each clap is band-passed noise bursts; swells in, fades out |
| `clap` | 0.70 s | 5,810 B | three 808-style hand claps (three quick noise bursts + a short tail) |
| `cheer` | 1.80 s | 14,940 B | nine "wooo!" voices (sawtooths with a rising-falling pitch through vowel formant filters) and two whistles, over light applause |
| `party_horn` | 1.20 s | 9,960 B | a party blower's reed: a pulse wave with a quick pitch rise and paper flutter; a short toot, then a long wobbly one |
| `tada` | 1.40 s | 11,620 B | a brass section (detuned sawtooths, the filter opening on the attack): a G chord stab, then a big C major chord |
| `drumroll` | 1.90 s | 15,770 B | a snare roll getting louder, then a snare + kick hit and an 808-style crash cymbal |
| `level_up` | 1.00 s | 8,300 B | an 8-bit square-wave arpeggio (C E G C E G) and a shimmering top note |
| `coins` | 1.50 s | 12,450 B | a two-note coin "bling" (B5 → E6), then a shower of ~40 small metal pings (inharmonic partials) |

The eight together: 93,790 bytes of flash.
