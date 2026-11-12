# Signal Desktop UI/UX parity

What Signal Desktop does and where this client stands. Voice and video calls are
deliberately out of scope for now.

Legend: ✅ done · 🚧 partial · ⬜ not yet · ✖ not supported (reason given)

## Chat list (left pane)

| Feature | Status |
|---|---|
| Navigation rail (Chats with unread count, Stories with new-story dot, Settings), 80px like `$NavTabs__width` | ✅ (no Calls tab: calls are out of scope) |
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
| Narrow windows: chat list collapses to avatars with unread badges; search expands it | ✅ |

## Conversation

| Feature | Status |
|---|---|
| Bubbles with sender grouping, date separators, delivery status | ✅ radius 18 / grouped 4, padding 12×8, as in Signal Desktop's stylesheets |
| Bubble width follows Signal Desktop's breakpoints (half the window when wide, 370px medium, 306px narrow) | ✅ |
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
| Message requests for unknown senders (accept / block / delete) | ✅ state read from Signal's storage service; answers are synced to your phone (MessageRequestResponse), which records them, as Signal Desktop does |
| Block / unblock contacts | ✅ the blocked list is synced both ways with your phone |
| Safety number view | ✅ |
| Leave group (header menu or group details, with confirmation); "no longer a member" banner | ✅ signed group change sent through libsignal-service's public push API |
| Group membership notices ("Bob left the group.", "Alice added Carol.", "Alice removed you.") | ✅ |
| Voice notes and audio attachments: in-app playback with progress | ✅ |
| Recording voice notes (mic button when the composer is empty) | ✅ MP3 via LAME at ~96 kbps mono, like Signal Desktop (which uses LAME at 90 kbps); WAV only if the encoder fails |

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
| Posting your own stories: text on a coloured background, or a photo/video, to "My Story" | ✅ sent to everyone you chat with, plus a sent-transcript so your phone shows it; stories you post from the phone appear here too |
| My Stories row, viewing your own stories | ✅ |
| Voice / video calls | out of scope |

## Verification

Everything above is covered by automated tests (unit, headless UI and offscreen rendering) on
macOS and Linux, and works in `--demo` mode. The network paths added most recently (leave
group, posting stories, storage-service state and the request/blocked/story syncs) are built
from Signal's protocol definitions and round-trip tested, but have not yet been exercised
against a live account.
