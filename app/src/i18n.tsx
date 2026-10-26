// Interface text in English and Turkish. The language follows the system
// unless the user picks one in Settings.

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

export type Language = "en" | "tr";
export type LanguageChoice = "system" | Language;

const en = {
  tagline: "Messages that get through when the internet doesn't.",
  yourName: "Your name",
  nameTooLong: "That name is too long for a contact card.",
  nameHint: "Your contacts see this name. There is no account: your identity is created and kept on this device.",
  createIdentity: "Create my identity",

  requests: "Requests",
  contacts: "Contacts",
  noMessagesYet: "No messages yet",
  sidebarEmpty: "Add the people you want to reach. Only one of you needs the other's card: your first message carries yours.",
  addContact: "Add contact",
  myCard: "My card",
  back: "Back",
  noConversation: "No conversation yet",
  noConversationHint:
    "Give your card to someone or add theirs. Messages wait on this device until a path appears.",

  wantsToMessage: (name: string) => `${name} wants to message you.`,
  declined: (name: string) => `Declined ${name}. Their messages are deleted and new ones will be ignored.`,
  exported: (count: number) =>
    `Saved ${count === 1 ? "1 item" : `${count} items`} to the file. Carry it to another CarryChat device.`,
  imported: (received: number, delivered: number, carried: number, skipped: number) =>
    `Imported: ${received === 1 ? "1 new message" : `${received} new messages`}, ` +
    `${delivered === 1 ? "1 delivery receipt" : `${delivered} delivery receipts`}, ${carried} to carry for others` +
    (skipped > 0 ? `, ${skipped} unreadable.` : "."),
  renamed: "Your contacts get your new name.",
  connected: (address: string) => `Connected to ${address}.`,

  compareFingerprint: "Compare this with what they see under My card",
  secure: "Forward secrecy",
  secureHint:
    "Keys change as you write back and forth. A key taken from a device later opens none of the messages before it.",
  securePending: "Forward secrecy starts with a reply",
  securePendingHint:
    "Until you have both written once, messages here use your pair key. A key taken from a device later could open them.",
  chatEmpty: (name: string) =>
    `Nothing here yet. What you write waits on this device until a path to ${name} appears: another CarryChat device on the network, the internet, or a file someone carries.`,
  waitingForPath: "Waiting for a path",
  delivered: "Delivered",
  read: "Read",
  messageTo: (name: string) => `Message ${name}`,
  messageGroup: (name: string) => `Message ${name}`,
  messageChannel: (name: string) => `Post to #${name}`,
  composerHint: "Enter sends, Shift+Enter starts a new line",
  tooLong: (bytes: number, max: number) => `Too long: ${bytes} of ${max} bytes`,
  onePacket: (bytes: number) => `${bytes} bytes · fits one radio packet`,
  packets: (bytes: number, count: number) => `${bytes} bytes · ${count} radio packets`,
  send: "Send",
  requestBar: (name: string) =>
    `${name} has your card and wrote to you first. Check this fingerprint with them before you accept:`,
  decline: "Decline",
  acceptAndReply: "Accept and reply",

  fix: "Fix…",
  details: "Details…",
  torSetting: "Reach the internet over Tor",
  torHint:
    "Relays then cannot see your IP address, and your network cannot see that you use CarryChat. Messages take a little longer. While Tor does not work, CarryChat does not use the internet at all; the local network and files still work.",
  restartToApply: "Applies when CarryChat starts again.",
  bluetoothSetting: "Find nearby devices over Bluetooth",
  bluetoothHint: "Phones within a few dozen metres pass messages to each other with no network at all.",
  restartNow: "Restart now",
  waitingToSend: (count: number) => `${count === 1 ? "1 message" : `${count} messages`} waiting to send`,
  carrying: (count: number) => `carrying ${count} for others`,
  preview: "Browser preview with sample data",

  addTitle: "Add a contact",
  theirCard: "Their card",
  addHint: "Paste the card they gave you. It starts with carrychat:. They don't need yours: your first message carries it.",
  cancel: "Cancel",
  add: "Add",
  addedTitle: (name: string) => `${name} is a contact`,
  addedCheck: (name: string) => `Before you rely on it, check that ${name} sees this fingerprint under My card:`,
  addedWarning: "If it differs, the card was changed on its way to you.",
  done: "Done",

  cardIntro:
    "Give this card to the people you want to hear from. They can write to you right away, and you see their first message as a request.",
  cardQr: "Scan with a phone camera, or copy the text below.",
  cardFingerprint: "Your fingerprint. Read it out to your contacts so they can check your card:",
  close: "Close",
  copyCard: "Copy card",
  copied: "Copied",

  connectTitle: "Connect to a device",
  connectIntro: "Devices on the same network usually find each other. If they don't, connect by address.",
  listensOn: (port: number) => ` This device listens on port ${port}.`,
  address: "Address",
  connectButton: "Connect",

  failedTitle: "The local network is not working",
  failedIntro: "CarryChat could not reach another device on this network:",
  failedHint:
    "Check that both devices are on the same network and that the other device's firewall lets CarryChat accept connections. Guest networks often keep devices from reaching each other.",
  blockedTitle: "Let CarryChat use the local network",
  blockedIntro: "macOS is not letting CarryChat reach other devices on this network, so messages cannot leave this Mac.",
  blockedStep1: "Open System Settings → Privacy & Security → Local Network.",
  blockedStep2: "Turn on CarryChat.",
  blockedStep3: "If this still shows as blocked after a few seconds, quit CarryChat and open it again.",
  openSystemSettings: "Open System Settings",

  save: "Save",
  renameHint: "Your contacts get your new name. Your fingerprint stays the same.",
  privacy: "Privacy",
  readReceipts: "Tell contacts when I read their messages",
  notifications: "Show a notification for new messages",
  language: "Language",
  languageSystem: "System",
  eraseTitle: "Erase everything",
  eraseHint: "Deletes your identity, contacts and messages from this device. It cannot be undone.",
  eraseButton: "Erase everything…",
  eraseConfirm: "Are you sure? CarryChat starts again empty.",
  eraseNow: "Erase now",

  groups: "Groups",
  newGroup: "New group",
  groupName: "Group name",
  groupNameTooLong: "That name is too long for a group.",
  groupWho: "Who is in it",
  groupNoContacts: "Add contacts first: a group is made of your contacts.",
  needsYourCard: "write to them once first",
  groupHint:
    "You become the group's admin: only you add and remove members. Up to 16 people, you included.",
  create: "Create",
  groupCreated: (name: string) => `Created ${name}. Members join as the group reaches them.`,
  membersCount: (count: number) => (count === 1 ? "1 member" : `${count} members`),
  membersTitle: (name: string) => `${name}: members`,
  you: "you",
  admin: "admin",
  remove: "Remove",
  messageMember: "Message",
  messageMemberHint: (name: string) =>
    `Adds ${name} to your contacts with the card the group gave you. Your first message reaches them as a request.`,
  memberAdded: (name: string) =>
    `${name} is in your contacts now. Compare fingerprints with them when you can.`,
  addMember: "Add",
  chooseContact: "Add a contact…",
  nobodyToAdd: "No one else to add",
  membersAdminHint:
    "Members you remove read nothing sent after the others hear of it. Only contacts who have your card can join.",
  membersHint: "Only the admin adds and removes members.",
  leaveGroup: "Leave group…",
  leaveNow: "Leave now",
  leftGroup: "You are not in this group any more. What was sent before stays here.",
  groupSecure: "Members only",
  channels: "Channels",
  joinChannel: "Join channel",
  channelName: "Channel name",
  channelPassword: "Password (optional)",
  channelNick: "Your nickname here",
  channelNameInvalid: "Use only letters, digits, - and _, up to 32 bytes.",
  joinHint:
    "Anyone who knows the name, and the password if there is one, reads and writes here. Your posts are signed with a key for this channel only, not with your card.",
  join: "Join",
  joinedChannel: (name: string) => `You joined #${name}.`,
  postingAs: (nick: string) => `as ${nick}`,
  channelPublic: "Public",
  channelProtected: "Password",
  channelPublicHint:
    "Anyone who knows this channel reads it, also later. No forward secrecy, no delivery receipts; posts live a day and travel 4 hops.",
  channelEmpty: (name: string) =>
    `Nothing in #${name} yet. Anyone who knows this name can read what you post here, also later. Posts live a day.`,
  nickSave: "Save",
  shareCard: "Share my card",
  shareCardHint:
    "Readers can then add you as a contact. It links your posts in this channel to your card, and nothing else.",
  cardShared: "Your card is shared in the channel.",
  leaveChannel: "Leave channel…",
  leaveChannelTitle: "Leave the channel",
  leaveChannelHint: "Leaving forgets the posts here. You can join again any time.",
  leaveChannelNow: "Leave and forget posts",
  leftChannel: (name: string) => `You left #${name}.`,
  sharedCard: (name: string) => `shared a card: ${name}`,
  youSharedCard: "You shared your card",
  addThisContact: "Add contact",
  mute: "Mute",
  muted: (nick: string) => `${nick} is muted in this channel.`,
  undo: "Undo",
  unmute: "Unmute",
  unmuted: (nick: string) => `${nick} is shown again. Posts you missed come with the next sync.`,
  mutedTitle: "Muted",
  attachPhoto: "Send a photo",
  recordVoice: "Record a voice message",
  stopRecording: "Stop and send",
  recording: (seconds: number) => `Recording… ${seconds} s, up to 2 minutes`,
  micDenied: "CarryChat cannot use the microphone. Allow it in System Settings → Privacy & Security → Microphone.",
  fileTooBig: "Too large: a file can be 1 MB at most.",
  photo: "Photo",
  voiceMessage: "Voice message",
  file: "File",
  fileArriving: (kind: string, received: number, chunks: number) => `${kind} arriving… ${received}/${chunks}`,
  fileLost: "This file did not arrive in time.",
  fileOther: (mime: string, kb: number) => `${mime}, ${kb} KB`,
  saveFile: "Save…",
  groupChatEmpty: (name: string) =>
    `Nothing in ${name} yet. A message here is one bundle that every member opens, and members carry it on for one another.`,
  tabChats: "Chats",
  tabNetwork: "Network",
  tabMe: "Me",
  search: "Search",
  filterAll: "All",
  filterUnread: "Unread",
  requestTag: "Request",
  chatsEmpty: "No conversations yet. Add someone with their card, or give them yours.",
  nothingFound: "Nothing found.",
  newTitle: "New",
  addContactHint: "With the card they gave you",
  newGroupHint: "One conversation with several contacts",
  joinChannelHint: "An open room you enter by its name",
  today: "Today",
  yesterday: "Yesterday",
  summaryNearby: (count: number) => (count === 1 ? "1 device nearby" : `${count} devices nearby`),
  summaryNoInternet: "No internet",
  summaryInternet: "Internet connected",
  summaryNothing: "No connection: messages wait on this device",
  summaryLanProblem: "Local network not working",
  networkSubtitle: "The ways your messages can travel",
  nearbyCount: (count: number) => (count === 1 ? "1 device nearby" : `${count} devices nearby`),
  nobodyNearby: "No devices nearby",
  nearbyOffline: "No internet, but your messages can still go from hand to hand.",
  nearbyOnline: "Messages go to nearby devices and over the internet.",
  internetOnly: "Messages go over the internet.",
  waitingHere: "Messages wait on this device until a path appears: a CarryChat device nearby, the internet or a file.",
  linksTitle: "Links",
  lanName: "Local network",
  bluetoothName: "Bluetooth",
  internetName: "Internet",
  torName: "Tor",
  torShort: "Hides your IP address from relays",
  stateOff: "Off",
  stateLooking: "Looking for devices",
  stateBlocked: "Blocked by macOS",
  stateFailed: "Not working",
  stateConnecting: "Connecting",
  stateRelays: (count: number) => (count === 1 ? "1 relay" : `${count} relays`),
  stateTorStarting: "Starting Tor…",
  stateTorFailed: "Tor did not start",
  byHandTitle: "By hand",
  exportRow: "Export to a file",
  exportRowHint: "Carry waiting messages on a USB stick",
  importRow: "Import a file",
  importRowHint: "A file from another CarryChat device",
  connectRow: "Connect to an address",
  connectRowHint: "Reach a device on another network",
  queueTitle: "On this device",
  myCardHint: "Show it so others can add you",
  theme: "Appearance",
  filterWaiting: "On the way",
  onTheWay: "On the way",
  loraName: "LoRa radio",
  fileName: "File",
  journeyTitle: "The message's journey",
  journeyWritten: "Written and encrypted",
  journeyWrittenHint: "On this device",
  journeyStep: (via: string) =>
    ({
      lan: "Left over the local network",
      bluetooth: "Left over Bluetooth",
      internet: "Went out to the internet",
      lora: "Sent by LoRa radio",
      file: "Written to a file",
    })[via] ?? via,
  journeyStepHint: (via: string, count: number) =>
    via === "internet"
      ? count === 1
        ? "Left on 1 relay"
        : `Left on ${count} relays`
      : via === "lora"
        ? "Handed to the radio"
        : via === "file"
          ? "Someone can carry it on"
          : count === 1
            ? "Handed to 1 device nearby"
            : `Handed to ${count} devices nearby`,
  journeyArrived: (name: string) => `Reached ${name}`,
  journeyNotYet: "Not arrived yet",
  journeyNotYetHint: "It goes on as soon as a path appears.",
  journeyCarriers: "Devices that carry it cannot read it; they only pass it on.",
  statOnTheWay: "on the way",
  statArrivedIn: "to arrive",
  statDevices: "devices nearby",
  statWithInternet: "Internet",
  statWithoutInternet: "No internet",
  statArrived: "arrived",
  duration: (seconds: number) => {
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);
    if (days > 0) return `${days} d ${hours % 24} h`;
    if (hours > 0) return `${hours} h ${minutes % 60} min`;
    return minutes > 0 ? `${minutes} min` : "< 1 min";
  },
  themeSystem: "System",
  themeLight: "Light",
  themeDark: "Dark",
};

type Strings = typeof en;

const tr: Strings = {
  tagline: "İnternet olmadığında da ulaşan mesajlar.",
  yourName: "Adın",
  nameTooLong: "Bu ad kişi kartı için fazla uzun.",
  nameHint: "Kişilerin bu adı görür. Hesap yok: kimliğin bu cihazda oluşturulur ve burada kalır.",
  createIdentity: "Kimliğimi oluştur",

  requests: "İstekler",
  contacts: "Kişiler",
  noMessagesYet: "Henüz mesaj yok",
  sidebarEmpty: "Ulaşmak istediğin kişileri ekle. Birinizin diğerinin kartına sahip olması yeter: ilk mesajın kendi kartını da taşır.",
  addContact: "Kişi ekle",
  myCard: "Kartım",
  back: "Geri",
  noConversation: "Henüz konuşma yok",
  noConversationHint: "Kartını birine ver ya da onunkini ekle. Mesajlar bir yol bulunana kadar bu cihazda bekler.",

  wantsToMessage: (name) => `${name} sana yazmak istiyor.`,
  declined: (name) => `${name} reddedildi. Mesajları silindi, yenileri yok sayılacak.`,
  exported: (count) => `${count} öğe dosyaya kaydedildi. Dosyayı başka bir CarryChat cihazına taşı.`,
  imported: (received, delivered, carried, skipped) =>
    `İçe aktarıldı: ${received} yeni mesaj, ${delivered} teslim kanıtı, başkaları için taşınacak ${carried} paket` +
    (skipped > 0 ? `, okunamayan ${skipped}.` : "."),
  renamed: "Kişilerin yeni adını alacak.",
  connected: (address) => `${address} adresine bağlanıldı.`,

  compareFingerprint: "Bunu onun Kartım ekranındakiyle karşılaştır",
  secure: "İleri gizlilik",
  secureHint:
    "Karşılıklı yazdıkça anahtarlar değişir. Sonradan bir cihazdan alınan anahtar, önceki mesajların hiçbirini açmaz.",
  securePending: "İleri gizlilik ilk cevapla başlar",
  securePendingHint:
    "İkiniz de birer kez yazana kadar mesajlar ortak anahtarınızla şifrelenir. Sonradan bir cihazdan alınan anahtar bunları açabilir.",
  chatEmpty: (name) =>
    `Henüz bir şey yok. Yazdıkların ${name} kişisine bir yol bulunana kadar bu cihazda bekler: ağdaki başka bir CarryChat cihazı, internet ya da birinin taşıdığı bir dosya.`,
  waitingForPath: "Yol bekleniyor",
  delivered: "Teslim edildi",
  read: "Okundu",
  messageTo: (name) => `${name} kişisine mesaj`,
  messageGroup: (name) => `${name} grubuna mesaj`,
  messageChannel: (name) => `#${name} kanalına yaz`,
  composerHint: "Enter gönderir, Shift+Enter yeni satır açar",
  tooLong: (bytes, max) => `Çok uzun: ${bytes}/${max} bayt`,
  onePacket: (bytes) => `${bytes} bayt · tek radyo paketine sığar`,
  packets: (bytes, count) => `${bytes} bayt · ${count} radyo paketi`,
  send: "Gönder",
  requestBar: (name) =>
    `${name} senin kartına sahip ve sana ilk o yazdı. Kabul etmeden önce bu parmak izini onunla karşılaştır:`,
  decline: "Reddet",
  acceptAndReply: "Kabul et ve yanıtla",

  fix: "Düzelt…",
  details: "Ayrıntılar…",
  torSetting: "İnternete Tor üzerinden bağlan",
  torHint:
    "Sunucular IP adresini, bağlı olduğun ağ da CarryChat kullandığını göremez. Mesajlar biraz daha geç gider. Tor çalışmadığı sürece CarryChat interneti hiç kullanmaz; yerel ağ ve dosyalar çalışmaya devam eder.",
  restartToApply: "CarryChat yeniden başlayınca geçerli olur.",
  bluetoothSetting: "Yakındaki cihazları Bluetooth ile bul",
  bluetoothHint: "Birkaç on metre içindeki telefonlar, hiç ağ olmadan da birbirine mesaj aktarır.",
  restartNow: "Şimdi yeniden başlat",
  waitingToSend: (count) => `${count} mesaj gönderilmeyi bekliyor`,
  carrying: (count) => `başkaları için ${count} paket taşınıyor`,
  preview: "Örnek verili tarayıcı önizlemesi",

  addTitle: "Kişi ekle",
  theirCard: "Kartı",
  addHint: "Sana verdiği kartı yapıştır. carrychat: ile başlar. Onun senin kartına ihtiyacı yok: ilk mesajın kartını da taşır.",
  cancel: "Vazgeç",
  add: "Ekle",
  addedTitle: (name) => `${name} artık bir kişi`,
  addedCheck: (name) => `Güvenmeden önce ${name} kişisinin Kartım ekranında bu parmak izini gördüğünü kontrol et:`,
  addedWarning: "Farklıysa kart sana ulaşırken değiştirilmiş demektir.",
  done: "Tamam",

  cardIntro:
    "Sana yazabilmesini istediğin kişilere bu kartı ver. Sana hemen yazabilirler, ilk mesajlarını bir istek olarak görürsün.",
  cardQr: "Telefon kamerasıyla tara ya da aşağıdaki metni kopyala.",
  cardFingerprint: "Parmak izin. Kişilerin kartını kontrol edebilsin diye onlara oku:",
  close: "Kapat",
  copyCard: "Kartı kopyala",
  copied: "Kopyalandı",

  connectTitle: "Bir cihaza bağlan",
  connectIntro: "Aynı ağdaki cihazlar genellikle birbirini bulur. Bulamazlarsa adresle bağlan.",
  listensOn: (port) => ` Bu cihaz ${port} portunu dinliyor.`,
  address: "Adres",
  connectButton: "Bağlan",

  failedTitle: "Yerel ağ çalışmıyor",
  failedIntro: "CarryChat bu ağdaki başka bir cihaza ulaşamadı:",
  failedHint:
    "İki cihazın aynı ağda olduğunu ve diğer cihazın güvenlik duvarının CarryChat'e izin verdiğini kontrol et. Misafir ağları çoğu zaman cihazların birbirine ulaşmasını engeller.",
  blockedTitle: "CarryChat'in yerel ağı kullanmasına izin ver",
  blockedIntro: "macOS, CarryChat'in bu ağdaki diğer cihazlara ulaşmasına izin vermiyor; mesajlar bu Mac'ten çıkamıyor.",
  blockedStep1: "Sistem Ayarları → Gizlilik ve Güvenlik → Yerel Ağ'ı aç.",
  blockedStep2: "CarryChat'i aç.",
  blockedStep3: "Birkaç saniye sonra hâlâ engelli görünüyorsa CarryChat'ten çık ve yeniden aç.",
  openSystemSettings: "Sistem Ayarları'nı aç",

  save: "Kaydet",
  renameHint: "Kişilerin yeni adını alır. Parmak izin değişmez.",
  privacy: "Gizlilik",
  readReceipts: "Mesajlarını okuduğumda kişilere bildir",
  notifications: "Yeni mesajlarda bildirim göster",
  language: "Dil",
  languageSystem: "Sistem",
  eraseTitle: "Her şeyi sil",
  eraseHint: "Kimliğini, kişilerini ve mesajlarını bu cihazdan siler. Geri alınamaz.",
  eraseButton: "Her şeyi sil…",
  eraseConfirm: "Emin misin? CarryChat boş olarak yeniden başlar.",
  eraseNow: "Şimdi sil",

  groups: "Gruplar",
  newGroup: "Yeni grup",
  groupName: "Grup adı",
  groupNameTooLong: "Bu ad grup için fazla uzun.",
  groupWho: "Kimler olacak",
  groupNoContacts: "Önce kişi ekle: grup kişilerinden oluşur.",
  needsYourCard: "önce ona bir kez yaz",
  groupHint: "Grubun yöneticisi sen olursun: üye ekleyip çıkaran yalnızca sensin. Sen dahil en fazla 16 kişi.",
  create: "Oluştur",
  groupCreated: (name) => `${name} oluşturuldu. Üyeler grup kendilerine ulaştıkça katılır.`,
  membersCount: (count) => `${count} üye`,
  membersTitle: (name) => `${name}: üyeler`,
  you: "sen",
  admin: "yönetici",
  remove: "Çıkar",
  messageMember: "Mesaj yaz",
  messageMemberHint: (name) =>
    `${name} kişisini grubun verdiği kartla kişilerine ekler. İlk mesajın ona istek olarak ulaşır.`,
  memberAdded: (name) => `${name} artık kişilerinde. Fırsat bulunca parmak izlerinizi karşılaştırın.`,
  addMember: "Ekle",
  chooseContact: "Bir kişi ekle…",
  nobodyToAdd: "Eklenecek başka kimse yok",
  membersAdminHint:
    "Çıkardığın üye, diğerleri bunu duyduktan sonra gönderilenleri okuyamaz. Yalnızca kartına sahip kişiler katılabilir.",
  membersHint: "Üye ekleyip çıkaran yalnızca yöneticidir.",
  leaveGroup: "Gruptan ayrıl…",
  leaveNow: "Şimdi ayrıl",
  leftGroup: "Artık bu grupta değilsin. Daha önce gönderilenler burada kalır.",
  groupSecure: "Yalnızca üyeler",
  channels: "Kanallar",
  joinChannel: "Kanala katıl",
  channelName: "Kanal adı",
  channelPassword: "Parola (isteğe bağlı)",
  channelNick: "Buradaki takma adın",
  channelNameInvalid: "Yalnızca harf, rakam, - ve _ kullan, en fazla 32 bayt.",
  joinHint:
    "Adı, varsa parolasını da bilen herkes burada okuyup yazabilir. Mesajların kartınla değil, yalnızca bu kanala özel bir anahtarla imzalanır.",
  join: "Katıl",
  joinedChannel: (name) => `#${name} kanalına katıldın.`,
  postingAs: (nick) => `${nick} olarak`,
  channelPublic: "Herkese açık",
  channelProtected: "Parolalı",
  channelPublicHint:
    "Bu kanalı bilen herkes okuyabilir, sonradan da. İleri gizlilik ve teslim bilgisi yok; mesajlar bir gün yaşar ve 4 adım gider.",
  channelEmpty: (name) =>
    `#${name} kanalında henüz bir şey yok. Bu adı bilen herkes burada yazdıklarını okuyabilir, sonradan da. Mesajlar bir gün yaşar.`,
  nickSave: "Kaydet",
  shareCard: "Kartımı paylaş",
  shareCardHint:
    "Okuyanlar seni kişi olarak ekleyebilir. Bu, yalnızca bu kanaldaki mesajlarını kartına bağlar, başka hiçbir şeyi değil.",
  cardShared: "Kartın kanalda paylaşıldı.",
  leaveChannel: "Kanaldan ayrıl…",
  leaveChannelTitle: "Kanaldan ayrıl",
  leaveChannelHint: "Ayrılınca buradaki mesajlar silinir. İstediğin zaman yeniden katılabilirsin.",
  leaveChannelNow: "Ayrıl ve mesajları sil",
  leftChannel: (name) => `#${name} kanalından ayrıldın.`,
  sharedCard: (name) => `bir kart paylaştı: ${name}`,
  youSharedCard: "Kartını paylaştın",
  addThisContact: "Kişi olarak ekle",
  mute: "Sustur",
  muted: (nick) => `${nick} bu kanalda susturuldu.`,
  undo: "Geri al",
  unmute: "Susturmayı kaldır",
  unmuted: (nick) => `${nick} yeniden görünüyor. Kaçırdığın mesajlar bir sonraki eşitlemede gelir.`,
  mutedTitle: "Susturulanlar",
  attachPhoto: "Fotoğraf gönder",
  recordVoice: "Sesli mesaj kaydet",
  stopRecording: "Durdur ve gönder",
  recording: (seconds) => `Kaydediliyor… ${seconds} sn, en fazla 2 dakika`,
  micDenied: "CarryChat mikrofonu kullanamıyor. Sistem Ayarları → Gizlilik ve Güvenlik → Mikrofon'dan izin ver.",
  fileTooBig: "Çok büyük: bir dosya en fazla 1 MB olabilir.",
  photo: "Fotoğraf",
  voiceMessage: "Sesli mesaj",
  file: "Dosya",
  fileArriving: (kind, received, chunks) => `${kind} geliyor… ${received}/${chunks}`,
  fileLost: "Bu dosya zamanında gelmedi.",
  fileOther: (mime, kb) => `${mime}, ${kb} KB`,
  saveFile: "Kaydet…",
  groupChatEmpty: (name) =>
    `${name} grubunda henüz bir şey yok. Buradaki her mesaj, bütün üyelerin açtığı tek bir pakettir ve üyeler onu birbirlerine taşır.`,
  tabChats: "Sohbetler",
  tabNetwork: "Ağ",
  tabMe: "Ben",
  search: "Ara",
  filterAll: "Tümü",
  filterUnread: "Okunmamış",
  requestTag: "İstek",
  chatsEmpty: "Henüz sohbet yok. Birini kartıyla ekle ya da ona kendi kartını ver.",
  nothingFound: "Bir şey bulunamadı.",
  newTitle: "Yeni",
  addContactHint: "Sana verdiği kartla",
  newGroupHint: "Birkaç kişiyle tek sohbet",
  joinChannelHint: "Adıyla girilen açık oda",
  today: "Bugün",
  yesterday: "Dün",
  summaryNearby: (count) => `yakında ${count} cihaz`,
  summaryNoInternet: "İnternet yok",
  summaryInternet: "İnternet bağlı",
  summaryNothing: "Bağlantı yok: mesajlar bu cihazda bekliyor",
  summaryLanProblem: "Yerel ağ çalışmıyor",
  networkSubtitle: "Mesajlarının gidebileceği yollar",
  nearbyCount: (count) => `${count} cihaz yakında`,
  nobodyNearby: "Yakında cihaz yok",
  nearbyOffline: "İnternet yok, mesajların yine de elden ele yola çıkabilir.",
  nearbyOnline: "Mesajlar yakındaki cihazlara ve internetten gidiyor.",
  internetOnly: "Mesajlar internetten gidiyor.",
  waitingHere: "Mesajlar bir yol bulunana kadar bu cihazda bekler: yakında bir CarryChat cihazı, internet ya da bir dosya.",
  linksTitle: "Bağlantılar",
  lanName: "Yerel ağ",
  bluetoothName: "Bluetooth",
  internetName: "İnternet",
  torName: "Tor",
  torShort: "IP adresini sunuculardan gizler",
  stateOff: "Kapalı",
  stateLooking: "Cihazlar aranıyor",
  stateBlocked: "macOS engelliyor",
  stateFailed: "Çalışmıyor",
  stateConnecting: "Bağlanıyor",
  stateRelays: (count) => `${count} sunucu`,
  stateTorStarting: "Tor başlıyor…",
  stateTorFailed: "Tor başlamadı",
  byHandTitle: "Elden ele",
  exportRow: "Dosyaya aktar",
  exportRowHint: "Bekleyen mesajları bellekle taşı",
  importRow: "Dosya al",
  importRowHint: "Başka bir CarryChat cihazından gelen dosya",
  connectRow: "Adrese bağlan",
  connectRowHint: "Başka bir ağdaki cihaza ulaş",
  queueTitle: "Bu cihazda",
  myCardHint: "Seni eklemeleri için göster",
  theme: "Görünüm",
  filterWaiting: "Yolda",
  onTheWay: "Yolda",
  loraName: "LoRa telsizi",
  fileName: "Dosya",
  journeyTitle: "Mesajın yolculuğu",
  journeyWritten: "Yazıldı ve şifrelendi",
  journeyWrittenHint: "Bu cihazda",
  journeyStep: (via) =>
    ({
      lan: "Yerel ağdan yola çıktı",
      bluetooth: "Bluetooth ile yola çıktı",
      internet: "İnternete çıktı",
      lora: "LoRa telsiziyle gönderildi",
      file: "Dosyaya yazıldı",
    })[via] ?? via,
  journeyStepHint: (via, count) =>
    via === "internet"
      ? `${count} sunucuya bırakıldı`
      : via === "lora"
        ? "Telsize verildi"
        : via === "file"
          ? "Biri onu taşıyabilir"
          : `Yakındaki ${count} cihaza verildi`,
  journeyArrived: (name) => `${name} kişisine ulaştı`,
  journeyNotYet: "Henüz ulaşmadı",
  journeyNotYetHint: "Bir yol bulununca devam edecek.",
  journeyCarriers: "Taşıyan cihazlar mesajı okuyamaz, yalnızca iletir.",
  statOnTheWay: "yolda",
  statArrivedIn: "sürede ulaştı",
  statDevices: "yakın cihaz",
  statWithInternet: "İnternetle",
  statWithoutInternet: "İnternetsiz",
  statArrived: "ulaştı",
  duration: (seconds) => {
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);
    if (days > 0) return `${days} gün ${hours % 24} sa`;
    if (hours > 0) return `${hours} sa ${minutes % 60} dk`;
    return minutes > 0 ? `${minutes} dk` : "1 dk'dan az";
  },
  themeSystem: "Sistem",
  themeLight: "Açık",
  themeDark: "Koyu",
};

const STORAGE_KEY = "carrychat.language";
const strings: Record<Language, Strings> = { en, tr };

function systemLanguage(): Language {
  return navigator.language.toLowerCase().startsWith("tr") ? "tr" : "en";
}

function storedChoice(): LanguageChoice {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored === "en" || stored === "tr" ? stored : "system";
  } catch {
    return "system";
  }
}

type I18n = {
  t: Strings;
  language: Language;
  choice: LanguageChoice;
  setChoice: (choice: LanguageChoice) => void;
};

const I18nContext = createContext<I18n | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [choice, setChoiceState] = useState<LanguageChoice>(storedChoice);
  const value = useMemo<I18n>(() => {
    const language = choice === "system" ? systemLanguage() : choice;
    return {
      t: strings[language],
      language,
      choice,
      setChoice(next) {
        try {
          if (next === "system") localStorage.removeItem(STORAGE_KEY);
          else localStorage.setItem(STORAGE_KEY, next);
        } catch {
          // Without storage the choice lasts until the window closes.
        }
        setChoiceState(next);
      },
    };
  }, [choice]);
  // Casing follows the page language: Turkish upper-cases i as İ.
  useEffect(() => {
    document.documentElement.lang = value.language;
  }, [value.language]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18n {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n needs an I18nProvider");
  return value;
}
