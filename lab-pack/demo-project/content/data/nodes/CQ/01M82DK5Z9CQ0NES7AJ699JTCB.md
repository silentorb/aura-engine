---
title: demo-01
created_at: 2026-11-22T20:01:38.793Z
modified_at: 2026-11-22T20:01:38.793Z
graph:
  nodes:
    0033AFBG0A003CRRHTZ1W36N77:
      id: 0033AFBG0A003CRRHTZ1W36N77
      type: time
      inputs: {}
      title: time
    0033AFBG0B003PN6DMP7VQVM7W:
      id: 0033AFBG0B003PN6DMP7VQVM7W
      type: constant_tempo
      inputs: {}
      title: tempo
    0033AFBG0C0040HM9EDDVCGK8H:
      id: 0033AFBG0C0040HM9EDDVCGK8H
      type: constant_time_signature
      inputs: {}
      title: meter
    0033AFBG0D004AE2584KV15J96:
      id: 0033AFBG0D004AE2584KV15J96
      type: epic_minor_progression
      inputs: {}
      title: prog
    0033AFBG0E004MAG11VSTNTH9V:
      id: 0033AFBG0E004MAG11VSTNTH9V
      type: loop
      inputs: {}
      title: loop_prog
    0033AFBG0F004Y6XWVJZTAFGAG:
      id: 0033AFBG0F004Y6XWVJZTAFGAG
      type: arpeggio
      inputs: {}
      title: arp
    0033AFBG0G00583BRNA5SZ4FB5:
      id: 0033AFBG0G00583BRNA5SZ4FB5
      type: loop
      inputs: {}
      title: loop_arp
    0033AFBG0H005HZSMF1BSKSEBT:
      id: 0033AFBG0H005HZSMF1BSKSEBT
      type: sine_note_voice
      inputs: {}
      title: arp_voice
    0033AFBG0J005VW7G8RHS8EDCF:
      id: 0033AFBG0J005VW7G8RHS8EDCF
      type: drum_grid
      inputs:
        lane: 0
      title: kick_grid
    0033AFBG0K0065RNC2FQRX3CD4:
      id: 0033AFBG0K0065RNC2FQRX3CD4
      type: loop
      inputs: {}
      title: loop_kick
    0033AFBG0M006FN37W6XRHRBDS:
      id: 0033AFBG0M006FN37W6XRHRBDS
      type: kick_drum
      inputs: {}
      title: kick
    0033AFBG0N006SHH3NY3R6DAEE:
      id: 0033AFBG0N006SHH3NY3R6DAEE
      type: drum_grid
      inputs:
        lane: 1
      title: snare_grid
    0033AFBG0P0073DYZFN9QV29F3:
      id: 0033AFBG0P0073DYZFN9QV29F3
      type: loop
      inputs: {}
      title: loop_snare
    0033AFBG0Q007DACV9CFQFQ8FR:
      id: 0033AFBG0Q007DACV9CFQFQ8FR
      type: snare_drum
      inputs: {}
      title: snare
    0033AFBG0R007Q6TQ33NQ4C7GD:
      id: 0033AFBG0R007Q6TQ33NQ4C7GD
      type: add
      inputs: {}
      title: mix
    0033AFBG0S008138JWTVPS16H2:
      id: 0033AFBG0S008138JWTVPS16H2
      type: bass_line
      inputs: {}
      title: bass
    0033AFBG0T008AZPEPJ1PDP5HQ:
      id: 0033AFBG0T008AZPEPJ1PDP5HQ
      type: loop
      inputs: {}
      title: loop_bass
    0033AFBG0V008MW4AG97P2B4JC:
      id: 0033AFBG0V008MW4AG97P2B4JC
      type: bass_saw_voice
      inputs: {}
      title: bass_voice
    0033AFBG0W008YRJ6A0DNQ03K1:
      id: 0033AFBG0W008YRJ6A0DNQ03K1
      type: output
      inputs: {}
      title: out
  edges:
    meter_to_prog:
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0D004AE2584KV15J96
        port: time_signature
    tempo_to_arp:
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: tempo
    meter_to_arp:
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: time_signature
    prog_to_loop_prog:
      from:
        node: 0033AFBG0D004AE2584KV15J96
        port: progression
      to:
        node: 0033AFBG0E004MAG11VSTNTH9V
        port: value
    loop_prog_to_arp:
      from:
        node: 0033AFBG0E004MAG11VSTNTH9V
        port: value
      to:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: progression
    arp_to_loop_arp:
      from:
        node: 0033AFBG0F004Y6XWVJZTAFGAG
        port: score
      to:
        node: 0033AFBG0G00583BRNA5SZ4FB5
        port: value
    loop_arp_to_arp_voice_score:
      from:
        node: 0033AFBG0G00583BRNA5SZ4FB5
        port: value
      to:
        node: 0033AFBG0H005HZSMF1BSKSEBT
        port: score
    time_to_arp_voice:
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0H005HZSMF1BSKSEBT
        port: time
    tempo_to_kick_grid:
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0J005VW7G8RHS8EDCF
        port: tempo
    meter_to_kick_grid:
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0J005VW7G8RHS8EDCF
        port: time_signature
    kick_grid_to_loop_kick:
      from:
        node: 0033AFBG0J005VW7G8RHS8EDCF
        port: score
      to:
        node: 0033AFBG0K0065RNC2FQRX3CD4
        port: value
    loop_kick_to_kick_score:
      from:
        node: 0033AFBG0K0065RNC2FQRX3CD4
        port: value
      to:
        node: 0033AFBG0M006FN37W6XRHRBDS
        port: score
    time_to_kick:
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0M006FN37W6XRHRBDS
        port: time
    tempo_to_snare_grid:
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0N006SHH3NY3R6DAEE
        port: tempo
    meter_to_snare_grid:
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0N006SHH3NY3R6DAEE
        port: time_signature
    snare_grid_to_loop_snare:
      from:
        node: 0033AFBG0N006SHH3NY3R6DAEE
        port: score
      to:
        node: 0033AFBG0P0073DYZFN9QV29F3
        port: value
    loop_snare_to_snare_score:
      from:
        node: 0033AFBG0P0073DYZFN9QV29F3
        port: value
      to:
        node: 0033AFBG0Q007DACV9CFQFQ8FR
        port: score
    time_to_snare:
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0Q007DACV9CFQFQ8FR
        port: time
    arp_voice_to_mix:
      from:
        node: 0033AFBG0H005HZSMF1BSKSEBT
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 0
    kick_to_mix:
      from:
        node: 0033AFBG0M006FN37W6XRHRBDS
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 1
    snare_to_mix:
      from:
        node: 0033AFBG0Q007DACV9CFQFQ8FR
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 2
    tempo_to_bass:
      from:
        node: 0033AFBG0B003PN6DMP7VQVM7W
        port: tempo
      to:
        node: 0033AFBG0S008138JWTVPS16H2
        port: tempo
    meter_to_bass:
      from:
        node: 0033AFBG0C0040HM9EDDVCGK8H
        port: time_signature
      to:
        node: 0033AFBG0S008138JWTVPS16H2
        port: time_signature
    loop_prog_to_bass:
      from:
        node: 0033AFBG0E004MAG11VSTNTH9V
        port: value
      to:
        node: 0033AFBG0S008138JWTVPS16H2
        port: progression
    bass_to_loop_bass:
      from:
        node: 0033AFBG0S008138JWTVPS16H2
        port: score
      to:
        node: 0033AFBG0T008AZPEPJ1PDP5HQ
        port: value
    loop_bass_to_bass_voice_score:
      from:
        node: 0033AFBG0T008AZPEPJ1PDP5HQ
        port: value
      to:
        node: 0033AFBG0V008MW4AG97P2B4JC
        port: score
    time_to_bass_voice:
      from:
        node: 0033AFBG0A003CRRHTZ1W36N77
        port: time
      to:
        node: 0033AFBG0V008MW4AG97P2B4JC
        port: time
    bass_voice_to_mix:
      from:
        node: 0033AFBG0V008MW4AG97P2B4JC
        port: audio
      to:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: inputs
      order: 3
    mix_to_out:
      from:
        node: 0033AFBG0R007Q6TQ33NQ4C7GD
        port: audio
      to:
        node: 0033AFBG0W008YRJ6A0DNQ03K1
        port: value
---
