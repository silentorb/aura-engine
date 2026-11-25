# Stereo: motivation

Why Aura carries stereo signals alongside mono, and why most node types are polymorphic over the two. Design: [`../design/audio-layouts.md`](../design/audio-layouts.md).

## Why now

Aura was mono end to end: a graph denoted `Time → Sample`, WAV export wrote one channel, and stereo plugins were folded to mono. That stopped being enough once third-party plugins arrived ([plugin hosting](plugin-hosting.md)): most instruments and effects are stereo, and folding them throws away their imaging. Panning and mixing parts across a stereo field is also basic arrangement work.

## Mono and stereo together

Workflows mix both. Oscillators, envelopes and most hand-built instruments are naturally mono; plugins, panned parts and the final mix are stereo. Aura supports both layouts and graphs that combine them, rather than making everything stereo.

## Polymorphic node types

Most DSP doesn't care how many channels it processes: a gain, a sum or a filter does the same thing to each channel. So most node types are **layout-polymorphic**: the layout of the input signal decides the layout of the output, as Imp generics (`(Signal<A>, Signal<A>) → Signal<A>`). A node type may still be exclusively mono or stereo (an oscillator is mono; a panner turns mono into stereo).

Inputs that share a type parameter must agree. A mono signal and a stereo signal into the same parameter is ambiguous (up-mix the mono? which way?), so for now it's an Imp **type error**, fixed by an explicit converter in the graph. Automatic promotion is a possible later step (see the plan's deferred note on join inference).

## Dual mono, prepared ahead of time

Each polymorphic DSP node type provides a mono function and may provide a stereo one. Without a stereo function, stereo runs the mono function once per channel ("dual mono"). The wrapper is prepared when the node type is loaded, not improvised during translation, so every runtime code path can rely on all implementations being present.

The same principle now applies to the streaming dimension: a node type's optional streaming implementation used to be looked up during translation, falling back to the deterministic one on a miss. Mono/stereo × deterministic/streaming form a 2×2 matrix, and once a node type is loaded all four cells exist.

## Shaping decisions

- Layout is a type, checked by Imp, not a runtime flag; mismatches are caught before translation.
- Dual mono is the default, so new effects get stereo for free; only genuinely stereo processing (panning, width, plugins) writes a second function.
- Stateful nodes (plugins) never run as dual mono: one instance processes both channels.
- Mono stays first-class; nothing is up-mixed behind the user's back.
