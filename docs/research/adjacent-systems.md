# Research: Adjacent Systems

This document captures the main lessons from nearby systems without turning
those systems into Fireline's architecture.

## Main takeaways

### 1. The real durability boundary matters more than the marketing layer

Many systems look durable inside one runtime but fragment at the first
cross-agent or cross-machine boundary.

Fireline's design should stay focused on where that boundary actually is.

### 2. Host-mediated agent communication is the common pattern

The strongest systems do not let agents authenticate and route to peers on
their own. The host mediates the call.

Fireline should keep that pattern.

### 3. Observation and replay should come from one durable source

Separate "live hub" and "catch-up API" systems usually drift. Fireline's
stream-first posture is the right simplification to keep.

### 4. Runtime lifecycle and protocol lifecycle are different concerns

A runtime can die while durable session data remains. A session can persist even
if the backing environment is replaced. Fireline should keep those concepts
separate.

## What Fireline should borrow

- host-mediated peer calls
- explicit runtime descriptors
- provider pinning
- attach vs connect separation
- one durable observation path

## What Fireline should avoid

- product-layer logic leaking into the substrate
- runtime-specific glue as the public API
- non-protocol side channels as the primary cross-node path
