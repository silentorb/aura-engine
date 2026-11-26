---
title: demo-01
created_at: 2026-11-22T20:01:38.793Z
modified_at: 2026-11-22T20:01:38.793Z
graph:
  nodes:
    - id: 0033AFBG0A003CRRHTZ1W36N77
      type: time
      inputs: {}
      title: time
    - id: 0033AFBG0B003PN6DMP7VQVM7W
      type: constant_tempo
      inputs: {}
      title: tempo
    - id: 0033AFBG0C0040HM9EDDVCGK8H
      type: constant_time_signature
      inputs: {}
      title: meter
    - id: 0033AFBG0D004AE2584KV15J96
      type: epic_minor_progression
      inputs: {}
      title: prog
    - id: 0033AFBG0E004MAG11VSTNTH9V
      type: loop
      inputs: {}
      title: loop_prog
    - id: 0033AFBG0F004Y6XWVJZTAFGAG
      type: arpeggio
      inputs: {}
      title: arp
    - id: 0033AFBG0G00583BRNA5SZ4FB5
      type: loop
      inputs: {}
      title: loop_arp
    - id: 0033AFBG0H005HZSMF1BSKSEBT
      type: sine_note_voice
      inputs: {}
      title: arp_voice
    - id: 0033AFBG0J005VW7G8RHS8EDCF
      type: drum_grid
      inputs:
        lane: 0
      title: kick_grid
    - id: 0033AFBG0K0065RNC2FQRX3CD4
      type: loop
      inputs: {}
      title: loop_kick
    - id: 0033AFBG0M006FN37W6XRHRBDS
      type: kick_drum
      inputs: {}
      title: kick
    - id: 0033AFBG0N006SHH3NY3R6DAEE
      type: drum_grid
      inputs:
        lane: 1
      title: snare_grid
    - id: 0033AFBG0P0073DYZFN9QV29F3
      type: loop
      inputs: {}
      title: loop_snare
    - id: 0033AFBG0Q007DACV9CFQFQ8FR
      type: snare_drum
      inputs: {}
      title: snare
    - id: 0033AFBG0R007Q6TQ33NQ4C7GD
      type: add
      inputs: {}
      title: mix
    - id: 0033AFBG0S008138JWTVPS16H2
      type: bass_line
      inputs: {}
      title: bass
    - id: 0033AFBG0T008AZPEPJ1PDP5HQ
      type: loop
      inputs: {}
      title: loop_bass
    - id: 0033AFBG0V008MW4AG97P2B4JC
      type: bass_saw_voice
      inputs: {}
      title: bass_voice
    - id: 0033AFBG0W008YRJ6A0DNQ03K1
      type: output
      inputs: {}
      title: out
  edges:
    - id: meter_to_prog
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0D004AE2584KV15J96
        port: time_signature
    - id: tempo_to_arp
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: tempo
    - id: meter_to_arp
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: time_signature
    - id: prog_to_loop_prog
      from:
        node: 0033AFBG0D004AE2584KV15J96
        port: progression
      to:
        node: 0033AFBG0E004MAG11VSTNTH9V
        port: value
    - id: loop_prog_to_arp
      from:
        node: 0033AFBG0E004MAG11VSTNTH9V
        port: value
      to:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: progression
    - id: arp_to_loop_arp
      from:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: score
      to:
        node: 0033AFBG0G00583BRNA5SZ4FB5
        port: value
    - id: loop_arp_to_arp_voice_score
      from:
        node: 0033AFBG0G00583BRNA5SZ4FB5
        port: value
      to:
        node: 0033AFBG0H005HZSMF1BSKSEBT
        port: score
    - id: time_to_arp_voice
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0H005HZSMF1BSKSEBT
        port: time
    - id: tempo_to_kick_grid
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0J005VW7G8RHS8EDCF
        port: tempo
    - id: meter_to_kick_grid
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0J005VW7G8RHS8EDCF
        port: time_signature
    - id: kick_grid_to_loop_kick
      from:
        node: 0033AFBG0J005VW7G8RHS8EDCF
        port: score
      to:
        node: 0033AFBG0K0065RNC2FQRX3CD4
        port: value
    - id: loop_kick_to_kick_score
      from:
        node: 0033AFBG0K0065RNC2FQRX3CD4
        port: value
      to:
        node: 0033AFBG0M006FN37W6XRHRBDS
        port: score
    - id: time_to_kick
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0M006FN37W6XRHRBDS
        port: time
    - id: tempo_to_snare_grid
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0N006SHH3NY3R6DAEE
        port: tempo
    - id: meter_to_snare_grid
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0N006SHH3NY3R6DAEE
        port: time_signature
    - id: snare_grid_to_loop_snare
      from:
        node: 0033AFBG0N006SHH3NY3R6DAEE
        port: score
      to:
        node: 0033AFBG0P0073DYZFN9QV29F3
        port: value
    - id: loop_snare_to_snare_score
      from:
        node: 0033AFBG0P0073DYZFN9QV29F3
        port: value
      to:
        node: 0033AFBG0Q007DACV9CFQFQ8FR
        port: score
    - id: time_to_snare
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0Q007DACV9CFQFQ8FR
        port: time
    - id: arp_voice_to_mix
      from:
        node: 0033AFBG0H005HZSMF1BSKSEBT
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 0
    - id: kick_to_mix
      from:
        node: 0033AFBG0M006FN37W6XRHRBDS
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 1
    - id: snare_to_mix
      from:
        node: 0033AFBG0Q007DACV9CFQFQ8FR
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 2
    - id: tempo_to_bass
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0S008138JWTVPS16H2
        port: tempo
    - id: meter_to_bass
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0S008138JWTVPS16H2
        port: time_signature
    - id: loop_prog_to_bass
      from:
        node: 0033AFBG0E004MAG11VSTNTH9V
        port: value
      to:
        node: 0033AFBG0S008138JWTVPS16H2
        port: progression
    - id: bass_to_loop_bass
      from:
        node: 0033AFBG0S008138JWTVPS16H2
        port: score
      to:
        node: 0033AFBG0T008AZPEPJ1PDP5HQ
        port: value
    - id: loop_bass_to_bass_voice_score
      from:
        node: 0033AFBG0T008AZPEPJ1PDP5HQ
        port: value
      to:
        node: 0033AFBG0V008MW4AG97P2B4JC
        port: score
    - id: time_to_bass_voice
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0V008MW4AG97P2B4JC
        port: time
    - id: bass_voice_to_mix
      from:
        node: 0033AFBG0V008MW4AG97P2B4JC
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 3
    - id: mix_to_out
      from:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: audio
      to:
        node: 0033AFBG0W008YRJ6A0DNQ03K1
        port: value
---
