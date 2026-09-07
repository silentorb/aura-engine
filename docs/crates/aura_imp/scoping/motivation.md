# aura_imp — scoping

Aura needs a declarative portable DAG for synthesis. Imp supplies `Graph`, node libraries, and registries. Stock `imp_execution` targets tabular collection/path evaluation — wrong for continuous audio — so Aura owns Time → Sample translation separately in `aura_integration`.

See also [`../../../scoping/background.md`](../../../scoping/background.md).
