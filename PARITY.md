# Signal Desktop UI/UX parity

What Signal Desktop does and where this client stands. Voice and video calls are
deliberately out of scope for now.

Legend: ✅ done · 🚧 partial · ⬜ not yet · ✖ not supported (reason given)

## Chat list (left pane)

| Feature | Status |
|---|---|
| Conversation list with avatar, title, preview, time, unread badge | ✅ |
| Pinned chats on top | ✅ |
| Search chats, contacts and message text | ✅ |
| New chat (contact picker), Note to Self | ✅ |
| Archived chats view | ✅ |
| Right-click menu on a chat: pin, mute, mark read/unread, archive, delete | ✅ |
| Mute for 1 hour / 8 hours / 1 day / 1 week / always | ✅ |
| Mark as unread (blue dot) | ✅ |
| Delete chat (local, with confirmation) | ✅ |
| Draft, typing, "Message request" and "Blocked" previews | ✅ |
| Mention and timer-notice previews with names | ✅ |
| Connection banner (connecting / offline) | ✅ |

## Conversation

| Feature | Status |
|---|---|
| Bubbles with sender grouping, date separators, delivery status | ✅ |
| Group sender names and avatars | ✅ |
| Header: chat details / group members, search, menu | ✅ |
| Hover actions: reaction bar (6 quick reactions + all emoji), reply, more | ✅ |
| Reactions: pills with counts; click to see who reacted | ✅ |
| Reply (quote); click a quote to jump to the original | ✅ |
| Edit own messages (24 h); "Edited" label; edit history in Info | ✅ |
| Delete for everyone; delete for me | ✅ |
| Forward to one or more chats | ✅ |
| Message info (times, status, reactions, attachments, edit history) | ✅ |
| Copy text | ✅ |
| Clickable links and link-preview cards | ✅ |
| Formatting (bold, italic, strikethrough, monospace, spoiler) and @mentions | ✅ |
| @mention autocomplete in group chats (sent as real mentions) | ✅ |
| View-once photos and videos (open once, then deleted) | ✅ |
| Contact and group photos; contact "about" and group description | ✅ |
| Unread messages divider; chats open at the first unread message | ✅ |
| Full date on timestamp hover | ✅ |
| Typing indicator | ✅ |
| Jump to latest | ✅ |
| Disappearing messages: timer setting, countdown, auto-delete, timer notices | ✅ |
| Image attachments inline; files open externally | ✅ |
| Stickers (shown as images), shared contacts | ✅ |
| Placeholder for unsupported message types (payments, polls, gift badges) | ✅ |
| Image viewer (lightbox) with Open and Save As | ✅ |
| Search within a conversation | ✅ |
| Message requests for unknown senders (accept / block / delete) | 🚧 detected from the contact sync; presage doesn't expose Signal's accepted state |
| Block / unblock contacts | ✅ |
| Safety number view | ✅ |
| Leave group | ✖ needs group-change requests; libsignal-service only fetches groups (its group HTTP calls are crate-private), so this needs a fork |
| Voice notes and audio attachments: in-app playback with progress | ✅ |
| Recording voice notes (mic button when the composer is empty) | 🚧 sent as 16 kHz WAV with the voice-note flag; Signal's apps record AAC |

## Composer

| Feature | Status |
|---|---|
| Multi-line input, Enter to send, Shift+Enter for a newline | ✅ |
| Enter-to-send preference (⌘/Ctrl+Enter sends when off) | ✅ |
| Attach files (picker) and drag and drop | ✅ |
| Emoji picker with search | ✅ |
| Reply and edit banners | ✅ |
| Drafts | ✅ |

## App

| Feature | Status |
|---|---|
| Device linking with QR code | ✅ |
| Light / dark / system theme, Signal accent colour | ✅ |
| Desktop notifications, unread count in title | ✅ |
| Preferences: notifications on/off, notification content, read receipts, typing indicators, Enter-to-send, theme | ✅ |
| Keyboard shortcuts and shortcuts dialog | ✅ |
| Profile: edit your name and about; linked-device info; unlink | ✅ |
| Zoom in / out / actual size (⌘/Ctrl + / − / 0), saved in preferences | ✅ |
| Stories: view contacts' text and photo stories (24 h), unviewed rings, viewer | ✅ |
| Replying to stories (text or emoji) | ✅ |
| Posting your own stories | ✖ needs story distribution-list sending, not in presage |
| Voice / video calls | out of scope |
