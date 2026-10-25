// Interface text in English and Turkish. The language follows the system
// unless the user picks one in Settings.

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

export type Language = "en" | "tr";
export type LanguageChoice = "system" | Language;

const en = {
  tagline: "Messages that get through when the internet doesn't.",
  yourName: "Your name",
  nameTooLong: "That name is too long for a contact card.",
  nameHint: "Your contacts see this name. There is no account: your identity is created and kept on this computer.",
  createIdentity: "Create my identity",

  requests: "Requests",
  contacts: "Contacts",
  noMessagesYet: "No messages yet",
  sidebarEmpty: "Add the people you want to reach. Only one of you needs the other's card: your first message carries yours.",
  addContact: "Add contact",
  myCard: "My card",
  settings: "Settings",
  noConversation: "No conversation yet",
  noConversationHint:
    "Give your card to someone or add theirs. Messages wait on this computer until a path appears.",

  wantsToMessage: (name: string) => `${name} wants to message you.`,
  declined: (name: string) => `Declined ${name}. Their messages are deleted and new ones will be ignored.`,
  exported: (count: number) =>
    `Saved ${count === 1 ? "1 item" : `${count} items`} to the file. Carry it to another Kervan computer.`,
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
    `Nothing here yet. What you write waits on this computer until a path to ${name} appears: another Kervan computer on the network, the internet, or a file someone carries.`,
  waitingForPath: "Waiting for a path",
  delivered: "Delivered",
  read: "Read",
  messageTo: (name: string) => `Message ${name}`,
  composerHint: "Enter sends, Shift+Enter starts a new line",
  tooLong: (bytes: number, max: number) => `Too long: ${bytes} of ${max} bytes`,
  onePacket: (bytes: number) => `${bytes} bytes · fits one radio packet`,
  packets: (bytes: number, count: number) => `${bytes} bytes · ${count} radio packets`,
  send: "Send",
  requestBar: (name: string) =>
    `${name} has your card and wrote to you first. Check this fingerprint with them before you accept:`,
  decline: "Decline",
  acceptAndReply: "Accept and reply",

  lanOff: "Local network: off",
  lanWaiting: "Local network: looking for other computers",
  lanUp: (count: number) => `Local network: ${count === 1 ? "1 computer" : `${count} computers`} nearby`,
  lanBlocked: "Local network: blocked by macOS",
  lanFailed: "Local network: not working",
  fix: "Fix…",
  details: "Details…",
  internetOff: "Internet: off",
  internetConnecting: "Internet: connecting",
  internetUp: (count: number) => `Internet: ${count === 1 ? "1 relay" : `${count} relays`}`,
  waitingToSend: (count: number) => `${count === 1 ? "1 message" : `${count} messages`} waiting to send`,
  carrying: (count: number) => `carrying ${count} for others`,
  preview: "Browser preview with sample data",
  connect: "Connect…",
  importFile: "Import file…",
  exportFile: "Export file…",

  addTitle: "Add a contact",
  theirCard: "Their card",
  addHint: "Paste the card they gave you. It starts with kervan:. They don't need yours: your first message carries it.",
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

  connectTitle: "Connect to a computer",
  connectIntro: "Computers on the same network usually find each other. If they don't, connect by address.",
  listensOn: (port: number) => ` This computer listens on port ${port}.`,
  address: "Address",
  connectButton: "Connect",

  failedTitle: "The local network is not working",
  failedIntro: "Kervan could not reach another computer on this network:",
  failedHint:
    "Check that both computers are on the same network and that the other computer's firewall lets Kervan accept connections. Guest networks often keep computers from reaching each other.",
  blockedTitle: "Let Kervan use the local network",
  blockedIntro: "macOS is not letting Kervan reach other computers on this network, so messages cannot leave this Mac.",
  blockedStep1: "Open System Settings → Privacy & Security → Local Network.",
  blockedStep2: "Turn on Kervan.",
  blockedStep3: "If this still shows as blocked after a few seconds, quit Kervan and open it again.",
  openSystemSettings: "Open System Settings",

  settingsTitle: "Settings",
  save: "Save",
  renameHint: "Your contacts get your new name. Your fingerprint stays the same.",
  privacy: "Privacy",
  readReceipts: "Tell contacts when I read their messages",
  notifications: "Show a notification for new messages",
  language: "Language",
  languageSystem: "System",
  eraseTitle: "Erase everything",
  eraseHint: "Deletes your identity, contacts and messages from this computer. It cannot be undone.",
  eraseButton: "Erase everything…",
  eraseConfirm: "Are you sure? Kervan starts again empty.",
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
  groupSecureHint:
    "One message reaches every member. Only members can read it, and no member can write in another's name.",
  groupChatEmpty: (name: string) =>
    `Nothing in ${name} yet. A message here is one bundle that every member opens, and members carry it on for one another.`,
};

type Strings = typeof en;

const tr: Strings = {
  tagline: "İnternet olmadığında da ulaşan mesajlar.",
  yourName: "Adın",
  nameTooLong: "Bu ad kişi kartı için fazla uzun.",
  nameHint: "Kişilerin bu adı görür. Hesap yok: kimliğin bu bilgisayarda oluşturulur ve burada kalır.",
  createIdentity: "Kimliğimi oluştur",

  requests: "İstekler",
  contacts: "Kişiler",
  noMessagesYet: "Henüz mesaj yok",
  sidebarEmpty: "Ulaşmak istediğin kişileri ekle. Birinizin diğerinin kartına sahip olması yeter: ilk mesajın kendi kartını da taşır.",
  addContact: "Kişi ekle",
  myCard: "Kartım",
  settings: "Ayarlar",
  noConversation: "Henüz konuşma yok",
  noConversationHint: "Kartını birine ver ya da onunkini ekle. Mesajlar bir yol bulunana kadar bu bilgisayarda bekler.",

  wantsToMessage: (name) => `${name} sana yazmak istiyor.`,
  declined: (name) => `${name} reddedildi. Mesajları silindi, yenileri yok sayılacak.`,
  exported: (count) => `${count} öğe dosyaya kaydedildi. Dosyayı başka bir Kervan bilgisayarına taşı.`,
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
    `Henüz bir şey yok. Yazdıkların ${name} kişisine bir yol bulunana kadar bu bilgisayarda bekler: ağdaki başka bir Kervan bilgisayarı, internet ya da birinin taşıdığı bir dosya.`,
  waitingForPath: "Yol bekleniyor",
  delivered: "Teslim edildi",
  read: "Okundu",
  messageTo: (name) => `${name} kişisine mesaj`,
  composerHint: "Enter gönderir, Shift+Enter yeni satır açar",
  tooLong: (bytes, max) => `Çok uzun: ${bytes}/${max} bayt`,
  onePacket: (bytes) => `${bytes} bayt · tek radyo paketine sığar`,
  packets: (bytes, count) => `${bytes} bayt · ${count} radyo paketi`,
  send: "Gönder",
  requestBar: (name) =>
    `${name} senin kartına sahip ve sana ilk o yazdı. Kabul etmeden önce bu parmak izini onunla karşılaştır:`,
  decline: "Reddet",
  acceptAndReply: "Kabul et ve yanıtla",

  lanOff: "Yerel ağ: kapalı",
  lanWaiting: "Yerel ağ: diğer bilgisayarlar aranıyor",
  lanUp: (count) => `Yerel ağ: yakında ${count} bilgisayar`,
  lanBlocked: "Yerel ağ: macOS engelliyor",
  lanFailed: "Yerel ağ: çalışmıyor",
  fix: "Düzelt…",
  details: "Ayrıntılar…",
  internetOff: "İnternet: kapalı",
  internetConnecting: "İnternet: bağlanıyor",
  internetUp: (count) => `İnternet: ${count} sunucu`,
  waitingToSend: (count) => `${count} mesaj gönderilmeyi bekliyor`,
  carrying: (count) => `başkaları için ${count} paket taşınıyor`,
  preview: "Örnek verili tarayıcı önizlemesi",
  connect: "Bağlan…",
  importFile: "Dosya al…",
  exportFile: "Dosyaya aktar…",

  addTitle: "Kişi ekle",
  theirCard: "Kartı",
  addHint: "Sana verdiği kartı yapıştır. kervan: ile başlar. Onun senin kartına ihtiyacı yok: ilk mesajın kartını da taşır.",
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

  connectTitle: "Bir bilgisayara bağlan",
  connectIntro: "Aynı ağdaki bilgisayarlar genellikle birbirini bulur. Bulamazlarsa adresle bağlan.",
  listensOn: (port) => ` Bu bilgisayar ${port} portunu dinliyor.`,
  address: "Adres",
  connectButton: "Bağlan",

  failedTitle: "Yerel ağ çalışmıyor",
  failedIntro: "Kervan bu ağdaki başka bir bilgisayara ulaşamadı:",
  failedHint:
    "İki bilgisayarın aynı ağda olduğunu ve diğer bilgisayarın güvenlik duvarının Kervan'a izin verdiğini kontrol et. Misafir ağları çoğu zaman bilgisayarların birbirine ulaşmasını engeller.",
  blockedTitle: "Kervan'ın yerel ağı kullanmasına izin ver",
  blockedIntro: "macOS, Kervan'ın bu ağdaki diğer bilgisayarlara ulaşmasına izin vermiyor; mesajlar bu Mac'ten çıkamıyor.",
  blockedStep1: "Sistem Ayarları → Gizlilik ve Güvenlik → Yerel Ağ'ı aç.",
  blockedStep2: "Kervan'ı aç.",
  blockedStep3: "Birkaç saniye sonra hâlâ engelli görünüyorsa Kervan'dan çık ve yeniden aç.",
  openSystemSettings: "Sistem Ayarları'nı aç",

  settingsTitle: "Ayarlar",
  save: "Kaydet",
  renameHint: "Kişilerin yeni adını alır. Parmak izin değişmez.",
  privacy: "Gizlilik",
  readReceipts: "Mesajlarını okuduğumda kişilere bildir",
  notifications: "Yeni mesajlarda bildirim göster",
  language: "Dil",
  languageSystem: "Sistem",
  eraseTitle: "Her şeyi sil",
  eraseHint: "Kimliğini, kişilerini ve mesajlarını bu bilgisayardan siler. Geri alınamaz.",
  eraseButton: "Her şeyi sil…",
  eraseConfirm: "Emin misin? Kervan boş olarak yeniden başlar.",
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
  groupSecureHint:
    "Tek bir mesaj bütün üyelere ulaşır. Onu yalnızca üyeler okuyabilir ve hiçbir üye başkası adına yazamaz.",
  groupChatEmpty: (name) =>
    `${name} grubunda henüz bir şey yok. Buradaki her mesaj, bütün üyelerin açtığı tek bir pakettir ve üyeler onu birbirlerine taşır.`,
};

const STORAGE_KEY = "kervan.language";
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
