# `bevy-mod-richter`

[![Build Status](https://travis-ci.org/cormac-obrien/richter.svg?branch=devel)](https://travis-ci.org/cormac-obrien/richter)

An extended port of components from the [Richter](https://github.com/cormac-obrien/richter) Quake engine as a collection
of extensible modules for the [Bevy](https://bevyengine.org/) ecosystem. Should be at feature parity with Richter but as
refactoring work is ongoing some things may be broken temporarily. Started as a quick project to make Richter run on
macOS, but ended up with over 8500 lines added and 7000 lines removed.

As a part of the port, I heavily refactored the code to work closer to how Bevy expects things to work. This means that I
can get features for free that in my [previous Quake-related project](https://github.com/eira-fransham/goeld) I had to
implement manually - specifically, tonemapping/HDR and pipelined rendering. The audio system has also been completely
overhauled, and I even implemented a rewrite of Bevy's default audio framework to allow adding custom DSP effects using
[`fundsp`](https://github.com/SamiPerttu/fundsp). The way that it is currently written is very different from
[`bevy_fundsp`](https://github.com/harudagondi/bevy_fundsp), which is essentially just a helper for writing DSP output
to a buffer and then sending that buffer to Bevy's normal audio systems. All audio is now heirarchically organised into
mixers, with each mixer having control over the internal audio processing. The use-case for this could be that game audio
could have reverb applied while the menu audio could be left unchanged. Mixers are just normal components and can be
accessed as such. The project for the fork of `bevy_audio` is at [`bevy-mod-dynamicaudio`](https://github.com/eira-fransham/bevy-mod-dynamicaudio). The audio effects in the build
of the game at time of writing are a subtle reverb and filter delay, but most importantly I have added a limiter so that
the game audio doesn't completly blow the speakers out when there are more than a couple of sounds playing at once.

![alt tag](content/bevy-mod-richter-rogue.gif)

### Goals

The ultimate goal is for the renderer, client-server interactions, server, input and console to be separate modules that can
be mixed and matched without requiring all of them. As Quake already has a client-server model even in singleplayer games,
once the client is its own separate system that only communicates with the server through regular networking methods it
should be possible to write game logic in Rust (and therefore any scripting layer that integrates with Bevy, such as Lua)
and still have regular Quake clients connect to it, instead of being restricted to QuakeC.

These goals are partially completed, as the audio, rendering and input handling are already separate plugins, although
there are still some remaining interdependence issues.

### Status

The console and post-processing are done using regular Bevy primitives, with the console being rendered using `bevy-ui`.
The world and client updates are still handled with a centralised struct instead of components, making it impossible for
regular systems to interact with it. The console and keybinding system has been updated to be much more extensible, and
command implementations are just regular systems which can access any resource or component. All rendering is done through
the Bevy rendergraph, although the rendering code itself is still mostly written by hand using wgpu, albeit in a much more
extensible way than the original Richter implementation.

Networking is untested since beginning the port, and I've been only using demos as a testcase. It is a priority to get this
working again once the client update code is ported to use the ECS. I haven't touched most of the networking code, so in
theory it should still work or only require minor changes.

I've implemented mod support outside of the original `id1` directory, although it is unlikely to work with Quake mods that
are not primarily based on Quake 1. The only non-Quake games that are confirmed to function are Rogue and Hipnotic.
I have run the entirety of the "Quake Done Quickest" demos (`qdqst`) so can confirm that the whole game can be loaded and
rendered.

A host of bugs and limitations from the original Richter were fixed. Inputs are no longer handled by an enum and you can
define your own `+action`/`-action` commands which can be bound.

There are still a couple of small pieces of code that use nightly Rust, but I hope to fix those soon.

### Running

```
cd /path/to/quake
# To run Quake 1 (id1 folder)
cargo +nightly run --release --manifest-path /path/to/bevy-mod-quake --bin quake-client -- --game [GAME_NAME]
# To run other games
cargo +nightly run --release --manifest-path /path/to/bevy-mod-quake --bin quake-client -- --game [GAME_NAME]
```

#### Feature checklist

- Networking
  - [x] NetQuake network protocol implementation (`sv_protocol 15`)
    - [x] Connection protocol implemented
    - [x] All in-game server commands handled
    - [x] Carryover between levels
  - [ ] FitzQuake extended protocol support (`sv_protocol 666`)
- Rendering
  - [x] Deferred dynamic lighting
  - [x] Particle effects
  - [x] Pipelined rendering
  - [x] Customizable UI
  - Brush model (`.bsp`) rendering
    - Textures
      - [x] Static textures
      - [x] Animated textures
      - [x] Alternate animated textures
      - [x] Liquid texture warping
      - [ ] Sky texture scrolling (currently partial support)
    - [x] Lightmaps
    - [x] Occlusion culling
  - Alias model (`.mdl`) rendering
    - [x] Keyframe animation
      - [x] Static keyframes
      - [x] Animated keyframes
    - [ ] Keyframe interpolation
    - [ ] Ambient lighting
    - [x] Viewmodel rendering
  - UI
    - [x] Console
    - [x] HUD
    - [x] Level intermissions
    - [x] On-screen messages
    - [x] Menus
- Sound
  - [x] Loading and playback
  - [x] Entity sound
  - [x] Ambient sound
  - [x] Spatial attenuation
  - [ ] Stereo spatialization (almost complete)
  - [x] Music
  - [x] Global effects, particularly lookahead-enabled limiting to prevent audio clipping
- Console
  - [x] Line editing
  - [x] History browsing
  - [x] Cvar modification
  - [x] Command execution
  - [x] Quake script file execution
- Demos
  - [x] Demo playback
  - [ ] Demo recording
- File formats
  - [x] BSP loader
  - [x] MDL loader
  - [x] SPR loader
  - [x] PAK archive extraction
  - [x] WAD archive extraction

## Legal

This software is released under the terms of the MIT License (see LICENSE.txt).

This project is in no way affiliated with id Software LLC, Bethesda Softworks LLC, or ZeniMax Media
Inc. Information regarding the Quake trademark can be found at Bethesda's [legal information
page](https://bethesda.net/en/document/legal-information).

Due to licensing restrictions, the data files necessary to run Quake cannot be distributed with this
package. `pak0.pak`, which contains the files for the first episode ("shareware Quake"), can be
retrieved from id's FTP server at `ftp://ftp.idsoftware.com/idstuff/quake`. The full game can be
purchased from a number of retailers including Steam and GOG.
