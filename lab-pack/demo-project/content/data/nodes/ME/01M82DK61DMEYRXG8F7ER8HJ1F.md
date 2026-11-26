---
title: sine
created_at: 2026-11-22T20:01:38.861Z
modified_at: 2026-11-22T20:01:38.861Z
graph:
  nodes:
    - id: 0033AFBG000009WDVSQ5ZMMZ0N
      type: time
      inputs: {}
      title: time
    - id: 0033AFBG01000KRVQKEBZ99Y1A
      type: parameter
      inputs:
        value: 440
      title: freq
    - id: 0033AFBG02000XN9KD5HYXYX1Z
      type: sine
      inputs: {}
      title: osc
    - id: 0033AFBG030017HQF6WQYJKW2M
      type: output
      inputs: {}
      title: out
  edges:
    - id: freq_to_osc
      from:
        node: 0033AFBG01000KRVQKEBZ99Y1A
        port: value
      to:
        node: 0033AFBG02000XN9KD5HYXYX1Z
        port: frequency
    - id: time_to_osc
      from:
        node: 0033AFBG000009WDVSQ5ZMMZ0N
        port: time
      to:
        node: 0033AFBG02000XN9KD5HYXYX1Z
        port: time
    - id: osc_to_out
      from:
        node: 0033AFBG02000XN9KD5HYXYX1Z
        port: audio
      to:
        node: 0033AFBG030017HQF6WQYJKW2M
        port: value
---
