# Work sandbox internals

Before `work.checkout` binds an Armature session to a durable work run, Den validates the client's BearWire compatibility manifest. Rejection happens before checkout so an incompatible image cannot mutate task or run state.

See [BearWire compatibility](bearwire-compatibility.md) for the compatibility and release contract.
