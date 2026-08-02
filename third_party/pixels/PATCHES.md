# Local patches

This directory contains the published `pixels` 0.17.2 source from upstream
commit `47c09b923d9a646fe6d71515edcc497156f4f356`.

It is patched locally because `Pixels::render_with` retries surface acquisition
inside an unbounded loop. A persistent `Outdated`, `Lost`, or `Suboptimal`
status can therefore trap the application's event-loop callback before it can
process a resize or consult the renderer's device-health callback.

Local changes:

- reconfigure and retry an outdated surface at most once, then skip the frame;
- return `Error::SurfaceLost` immediately because wgpu requires the surface to
  be recreated rather than merely reconfigured;
- reconfigure a suboptimal surface at most once, then use the valid acquired
  frame if it remains suboptimal;
- cover the bounded retry policy with GPU-independent unit tests that run as
  part of the root workspace gates; and
- disable upstream doctests because the published crate omits their
  unpublished `pixels_mocks` helper.

Upstream tracking issue: <https://github.com/parasyte/pixels/issues/460>
