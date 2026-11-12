# Parity with Signal Desktop

This file compares Halyard with Signal Desktop. Video calls and group calls are out of scope.

All the features below are complete (✅). Notes show differences and limits.

## Chat list

- Navigation rail: Chats (with unread count), Stories (with new-story dot) and Settings.
  It has no Calls tab; call history shows in each chat.
- Chat list with avatars, previews, times and unread badges. Pinned chats are at the top.
- Search for chats, contacts and message text. New chat, Note to Self and archived chats.
- Chat menu: pin, mute (1 hour to always), mark read or unread, archive and delete.
- Previews for drafts, typing, message requests, blocked chats, mentions and notices.
- Connection banner.
- In narrow windows, the chat list collapses to avatars.

## Conversation

- Message bubbles with sender groups, date separators and status icons. The sizes, icons and
  width breakpoints come from Signal Desktop's stylesheets.
- Reactions, replies, edits (24 hours) with history, deletion for everyone or for you,
  forwarding, message information and copy.
- Links, link previews, formatting, @mentions with autocomplete and view-once media.
- Contact and group photos, contact "about" text and group descriptions.
- Unread divider, typing indicator, jump to latest and search in the chat.
- Disappearing messages: timer, countdown, automatic deletion and notices.
- Images, files, stickers and shared contacts. Image viewer with Save As.
- A placeholder shows for unsupported content (payments, polls and gift badges).
- Message requests: accept, block and delete. Halyard reads the state from Signal's storage
  service and syncs your answers to your phone.
- Block and unblock. Halyard syncs the blocked list with your phone in both directions.
- Safety numbers.
- Leave a group. Notices show when members join, leave or are removed.
- Voice messages: playback with progress, and recording as MP3 (LAME, about 96 kbps).

## Voice calls

- 1:1 voice calls with RingRTC, the call library of Signal's own apps. Audio is end-to-end
  encrypted. Signal's TURN relays connect calls through firewalls. Calls with people who are
  not your Signal connections always go through a relay, to hide your IP address.
- Call button in the chat header. Incoming-call bar with Accept and Decline, a desktop
  notification and Signal Desktop's ringtone.
- Call screen: name, photo, status (Calling, Ringing, Connecting, time), mute and hang up.
  It shows when the other person turns off their microphone.
- Call history in the chat (incoming, outgoing, missed, declined, unanswered), with
  "Call back". Missed calls count as unread.
- Calls from blocked chats and unanswered message requests do not ring. Halyard tells the caller
  that you have not accepted them, as Signal Desktop does.
- If someone offers a video call, Halyard answers with voice only.
- Ringtones (not in Signal Desktop): Signal's ringtone, five more (Classic, Chime, Marimba,
  Pulse, Sci-fi piano) or Silent. Preferences plays a preview when you choose one.

## Composer

- Multi-line text. Enter sends and Shift+Enter adds a new line (you can change this).
- Attachments from a file picker or with drag and drop. Emoji picker with search.
- Reply and edit banners. Drafts.

## App

- Device linking with a QR code. Profile name and "about" text. Unlink.
- Light, dark and system themes. Desktop notifications. Unread count in the window title.
- Preferences: notifications, notification content, read receipts, typing indicators and
  Enter to send.
- Keyboard shortcuts and a shortcuts dialog. Zoom (⌘/Ctrl + / − / 0).
- Stories: view, reply and post (text or photo, to "My Story"). Halyard syncs stories you
  post to your phone, and shows stories that you post from your phone.

## Verification

Automated tests on macOS and Linux and the demo mode cover all features.
Some network features have had no tests with a real account yet:

- leaving a group;
- posting stories;
- reading the storage service;
- syncing message requests, blocks and stories to your phone;
- voice calls with a phone. A test makes real RingRTC calls between two Halyard call engines,
  through Halyard's call message encoding. Calls with Signal's apps and relays are not tested yet.

Halyard builds these from Signal's protocol definitions, and round-trip tests cover them.
