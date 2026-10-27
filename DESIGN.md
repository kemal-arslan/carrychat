---
# gstack: design-md-format=spec
name: CarryChat
description: A light, quiet messenger. White surfaces, one blue accent, one typeface, and the few things only CarryChat shows (how a message travels, which links are open) drawn in the same plain language.
colors:
  accent: "#2F5BFF"
  on-accent: "#FFFFFF"
  accent-tint: "#E6ECFF"
  surface: "#FFFFFF"
  surface-muted: "#F3F4F6"
  line: "#EDEEF1"
  text: "#111216"
  text-muted: "#6B6F78"
  text-faint: "#A3A7AF"
  success: "#16A34A"
  error: "#DC2626"
  accent-night: "#2F5BFF"
  accent-text-night: "#7B96FF"
  accent-tint-night: "#1B2340"
  surface-night: "#121214"
  surface-muted-night: "#1D1D20"
  line-night: "#232327"
  text-night: "#F2F2F3"
  text-muted-night: "#9A9BA1"
  text-faint-night: "#5F6067"
  success-night: "#4ADE80"
  error-night: "#F87171"
typography:
  title:
    fontFamily: Figtree
    fontWeight: 750
    fontSize: 28px
    letterSpacing: -0.03em
  headline:
    fontFamily: Figtree
    fontWeight: 750
    fontSize: 20px
    letterSpacing: -0.02em
  body:
    fontFamily: Figtree
    fontSize: 15px
    lineHeight: 1.4
  body-phone:
    fontFamily: Figtree
    fontSize: 16px
    lineHeight: 1.4
  label:
    fontFamily: Figtree
    fontWeight: 650
    fontSize: 13px
  caption:
    fontFamily: Figtree
    fontSize: 12px
    fontFeature: tnum
rounded:
  xs: 6px
  sm: 12px
  md: 18px
  lg: 20px
  xl: 28px
  full: 9999px
spacing:
  xs: 4px
  sm: 8px
  md: 12px
  lg: 16px
  xl: 20px
  2xl: 32px
  touch: 44px
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.full}"
  button-secondary:
    backgroundColor: "{colors.surface-muted}"
    textColor: "{colors.text}"
    rounded: "{rounded.full}"
  input:
    backgroundColor: "{colors.surface-muted}"
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
  bubble-outgoing:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.lg}"
  bubble-incoming:
    backgroundColor: "{colors.surface-muted}"
    textColor: "{colors.text}"
    rounded: "{rounded.lg}"
  chip:
    backgroundColor: "{colors.surface-muted}"
    textColor: "{colors.text-muted}"
    rounded: "{rounded.full}"
  chip-active:
    backgroundColor: "{colors.text}"
    textColor: "{colors.surface}"
    rounded: "{rounded.full}"
  nav-indicator:
    backgroundColor: "{colors.accent-tint}"
    textColor: "{colors.accent}"
    rounded: "{rounded.full}"
  unread-badge:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.full}"
  fab:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.md}"
  sheet:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.xl}"
  group:
    borderColor: "{colors.line}"
    rounded: "{rounded.lg}"
---

# CarryChat

**English** · [Türkçe](DESIGN.tr.md)

## Overview

**Creative North Star:** a clean phone app that stays out of the way. People open CarryChat to read and send a few words, often on a weak battery and without internet; the interface should feel familiar within seconds and never look alarming.
**Product context:** an offline-first, end-to-end encrypted messenger for Android, macOS, Windows and Linux. Messages are stored and carried device to device, over the local network, Bluetooth, LoRa, the internet or a file, until a path to the recipient appears. They may arrive in seconds or in days.
**Mode per surface:** Operate everywhere: the chat list, a conversation, the composer, contacts, the network tab, settings and sheets.
**Reference sites:** current mobile messenger work on dribbble.com (light chat lists, floating action buttons, filter chips, bottom sheets), and Material 3 navigation on Android.
**Key characteristics:**
- White and light grey, one accent, no gradients, glows or glass.
- One typeface for everything.
- A waiting message looks like it is waiting, never like it failed.
- The links (local network, Bluetooth, internet) are a tab of their own, not a status bar.
- Dialogs are bottom sheets that can be dragged closed.

## Colors

**Strategy:** Restrained. Neutrals carry the interface; the accent marks what can be acted on and what is mine: primary buttons, the outgoing bubble, unread counts, the active tab and the floating button. Success green appears only for a live link ("2 devices nearby") and a verified contact. Error red is kept for real failures (a link that does not work, a message too long), never for a message that is waiting.
**Light or dark:** the app follows the system unless the user picks Light or Dark under Me, and on Android the status and navigation bars take the same surface colour. Light is the primary design: it reads well outdoors and matches what people expect from a messenger. The night theme uses flat dark greys (no tint, no gradient) and keeps the same hierarchy: surface, then surface-muted for bubbles and fields, then line for separators. The accent fill stays the same so white text on it keeps its contrast; accent used as text on the dark surface switches to the lighter accent-text-night.
White on the accent is 5.2:1. Text-muted on surface is 5.0:1; text-faint is for timestamps and placeholders only.

## Typography

- **Figtree**, variable, everywhere: titles at 750, names at 650, body at 400. It is a plain geometric sans that stays friendly at small sizes and covers Turkish completely (ç ğ ı İ ö ş ü).
- Titles are large (28px) and tight; there is no second display face.
- Message text is 16px on phones and 15px on desktops.
- Times and counts use tabular figures. Fingerprints use the system monospace font.
- Sentence case everywhere; no all-caps labels.
- Figtree is under the SIL Open Font License and ships inside the app through `@fontsource-variable/figtree`, so it looks the same with no internet and under the app's `font-src 'self'` policy.

## Layout

- **Phone (640px and below):** one screen at a time.
  - A bottom navigation bar (Material 3) with four destinations: Chats, People, Network, Me. The active one has a tinted pill behind a filled icon.
  - Opening a conversation covers the tab and hides the bar; Android's back key and the back arrow return.
  - Android's back key closes what is on top: a sheet, then the conversation, then any tab other than Chats. On Chats it sends the app to the background, as Home does.
  - A floating button sits above the bar on Chats (start something new) and People (add a contact).
- **Desktop:** the same four destinations live in a 360px column on the left, with the navigation bar at its foot; the open conversation fills the rest.
- **Chats:** large title with search and "my card" buttons, a one-line link summary that opens Network, filter chips (All, Unread, Groups, Channels), then rows with no separators: a 50px avatar, name, time, last message and unread count.
- **Conversation:** a header with back, avatar, name and one line of context, then bubbles, then a single-row composer.
- **Network:** a summary of how many devices are nearby, then grouped rows for each link with its state and switch, then the hand-carried options (export, import, connect by address).
- **Me:** the identity (name, fingerprint, card), then settings in grouped rows.
- Gutters are 20px on phones; rows are at least 44px tall; spacing steps in 4px.

## Elevation & Depth

Flat. Surfaces are told apart by fill (surface vs surface-muted) and hairlines (line), not shadows. Two exceptions, both with a soft offset shadow: the floating action button, which floats over the list, and sheets, which rise over a dimmed scrim.

## Shapes

- Bubbles are rounded at 20px, with the corner nearest the sender at 6px on the last bubble of a run.
- Grouped rows, cards and the network summary use 20px; text fields 12px; the floating button 18px; sheets 28px at the top.
- Buttons, chips, the composer field and badges are full pills. Avatars are circles: a light tint of a colour picked from the name, with the initial in that colour. Groups show a people icon, channels "#".

## Components

- **Buttons:** primary (accent fill), secondary (surface-muted fill), danger (error text on surface-muted; solid error only for the final confirmation). Focus-visible draws a 2px accent ring offset by 2px; pressed darkens by 6%; disabled drops to 40% opacity.
- **Message states** (outgoing):
  - Waiting for a path: the bubble is white with a dashed accent outline, and its footer says "Waiting for a path" with an hourglass. It is calm, not an error. Once it has left by some link but not arrived, the footer says "On the way".
  - Delivered: a single check. Read: a double check.
  - The footer shows a small icon for each link the message left by, in the order first used.
- **Message journey:** tapping one of our bubbles opens a sheet with how long it was on the way, how many nearby devices took it, whether the internet was used, and a timeline: written, each link it left by with the number of devices or relays and the time, arrived, read. A message that has not arrived ends the timeline with a dashed hourglass node.
- **On the way:** the chat list has a filter for conversations with messages that have not arrived, which show an hourglass before their last message.
- **Composer:** an attach button, a pill-shaped field that grows to five lines, and one round accent button that is a microphone while the field is empty and a send arrow once there is text. The packet count appears above the field only when the text needs more than one radio packet, or when something is wrong. In a chat with a contact, a speaker button before the field sends by sound too: filled and in the accent colour while on, with a line above the field that the other device must listen, and "Playing as sound…" while a message plays.
- **Sound:** a Network row with a switch, which listens while the app is open; its state reads "Listening while the app is open", or in the error colour when the microphone could not be used.
- **Link summary** (chat list): a coloured dot (green when any link is open, grey when none) and one sentence, such as "No internet · 3 devices nearby".
- **Network rows:** icon tile, name, state in words (green when live), and a switch where the link can be turned on or off. A link whose radio is off says what to turn on, such as "Turn on Bluetooth to use it". Changes that need a restart show one restart row.
- **LoRa:** its row opens a sheet. With no device chosen, the sheet lists Meshtastic and MeshCore devices to choose from: found over Bluetooth on phones, USB ports on computers, or an address on the network, with the port each kind listens on. With one chosen, it shows the device's name, state and kind, a Meshtastic device's region (in the error colour when unset, with how to set it) or a MeshCore device's radio settings, a note in the error colour when a MeshCore device has no free channel slot, when CarryChat last came over LoRa, the devices heard recently (three hours for Meshtastic, two days for MeshCore) with how long ago, hops and SNR, and a button to stop using it.
- **Reticulum:** its row opens a sheet. With no node chosen, it says what Reticulum is and takes a node's address. With one chosen, it shows the address and state, when CarryChat last came over Reticulum, the CarryChat nodes heard in the last six hours with how long ago and how many hops away, this node's own destination, and a button to stop using it.
- **Sheets:** every dialog is a bottom sheet with a grab handle, a 20px title and its actions at the bottom, full width on phones and 440px wide, centred, on desktops. They close by dragging down, tapping the scrim, Escape or Android's back key.
- **Empty states** say what to do next in one sentence.
- **People nearby:** a "Be visible nearby" switch on People. While it is on, People lists the visible people nearby with the link they are on, and Chats shows them as a row of ringed avatars above the filters; tapping one opens a chat, adding them as a contact first. Visibility is mutual and off by default.

## Do's and Don'ts

- Do: use the accent only for actions, outgoing bubbles, unread counts and the active tab.
- Do: show a waiting message with the dashed outline and the hourglass, in calm words.
- Do: use Phosphor icons, regular weight, and the fill weight for the active tab.
- Do: bundle the font; never load anything from the network at runtime.
- Do: keep every dialog a sheet that can be dragged closed.
- Don't: add gradients, glows, blurred glass or coloured backgrounds behind screens.
- Don't: introduce a second accent or a second typeface.
- Don't: show a waiting message in red or as failed.
- Don't: put shadows on rows, cards or bubbles.
- Don't: set labels in all caps or tracked tiny type.

## Motion

- **Approach:** minimal-functional.
- **Easing:** enter(ease-out) exit(ease-in) move(ease-in-out)
- **Duration:** micro(80ms) short(200ms) medium(300ms) long(500ms)
- **Screens:** opening a conversation on a phone slides it in from the right in 220ms. Sheets follow the finger and settle in 300ms.
- **The one authored moment:** when a waiting message is delivered, its dashed outline fills in to the solid accent bubble over 200ms.
- With "reduce motion" on, everything changes without movement.

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-10-26 | Initial design system created | Created by /design-consultation from the product context and a review of current mobile messenger designs; an orienteering-map direction and a dark warm direction were tried and set aside in favour of a light, plain interface that feels familiar on a phone. |
