# Interaction research

Research date: 2026-11-26. Scope: pointer, keyboard, and state feedback in the
GPUI UI. The change applies one state model to every control rather than giving
each screen its own hover rules.

## What the guidance says

Microinteractions are trigger and feedback pairs. Their job is to confirm that
an action registered, show system state, and prevent errors ([NN/g](https://www.nngroup.com/articles/microinteractions/)).
A hover state tells a pointer user that a control can be clicked. NN/g recommends
a subtle state change and notes that hover is invisible to keyboard and touch
users, so it cannot be the only signal ([NN/g](https://www.nngroup.com/articles/button-states-communicate-interaction/)).
Material describes states as semi-transparent layers over a component, with
separate hover, focus, pressed, and disabled treatments ([Material 3](https://m3.material.io/foundations/interaction/states/state-layers)).
Pointer conventions also use secondary click for context menus and keep the
cursor visible over interactive elements ([Material 3](https://m3.material.io/foundations/interaction/inputs)).
Apple's pointer guidance likewise expects the control's appearance to change
when a pointer hovers it ([Apple HIG](https://developer.apple.com/design/human-interface-guidelines/buttons)).

Macro interactions are the larger tasks a control serves. For this app the
important flows are choosing a deck, opening its actions, confirming a
destructive choice, and leaving that context. Keyboard users need a visible
focus indicator and must be able to reach every action ([WCAG 2.4.7](https://www.w3.org/WAI/WCAG21/Understanding/focus-visible.html)).
Secondary content can stay hidden, but it should appear when the user shows
intent, and it should not trap them ([NN/g](https://www.nngroup.com/articles/timing-exposing-content/)).

## Decisions

- One `Interaction` value supplies rest border, hover fill, pressed fill, and
  focus border. The border is reserved at rest, so a focus ring never shifts
  layout. Filled buttons use the same model with accent shades.
- Every clickable element owns a unique GPUI element id. Repeated ids made
  several rows share one hover state, which is why some actions looked inert.
- Disabled controls keep a muted ink and receive no hover, press, focus, or
  pointer affordance. They do not pretend to be live.
- Selected controls keep their selected fill at rest. Hover and press layer a
  stronger shade over it, so selection remains visible while interacting.
- Destructive menu actions use a red hover layer rather than no state change.
- Icon-only controls get a focus ring and a pressed state because they have no
  text label to carry feedback.
- The deck menu closes on outside press, on its trigger, on Escape, and on any
  dispatched action. An outside press records its mouse-down event so the same
  press cannot close the menu and then reopen it on trigger click. The record is
  cleared on the next press and on mouse-up even when no click fires.
- Hover changes color only. Motion stays limited to the existing fade and
  progress animations and respects the reduced-motion setting.

The result is deliberately quiet. Hover feedback is strong enough to notice
while moving across the UI, but subtle enough that ordinary pointer travel does
not make the screen flicker.
