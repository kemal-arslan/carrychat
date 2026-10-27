# CarryChat

[English](README.md) · **Türkçe**

Bu, İngilizce README'nin çevirisidir. İkisi arasında fark olursa İngilizcesi geçerlidir.

**Telefonlar ve bilgisayarlar arasında, internet olmadan da çalışan, uçtan uca şifreli mesajlaşma. Her bağlantıdan geçer: yerel ağ, Bluetooth, Wi-Fi Aware, Wi-Fi Direct, LoRa telsizi, Reticulum, internet ya da USB bellekteki bir dosya.**

CarryChat sakla ve ilet (store-and-forward) mantığıyla çalışan bir mesajlaşma uygulamasıdır. Her mesaj en fazla 4 KiB'lık bir zarfa mühürlenir, alıcıya doğru bir bağlantı çıkana kadar saklanır ve yol boyunca cihazdan cihaza aktarılır. Zarfı taşıyan cihazlar onu okuyamaz, kimden geldiğini ya da kime gittiğini de anlayamaz. İnternet bağlantılardan yalnızca biridir, şart değildir.

[![CI](https://github.com/kemal-arslan/carrychat/actions/workflows/ci.yml/badge.svg)](https://github.com/kemal-arslan/carrychat/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20me%20a%20coffee-support-FFDD00?logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/kemalarslan)

> **Durum: erken.** CarryChat bugün yerel ağ, Bluetooth, internet (Nostr aktarıcıları) ve dosya üzerinden; masaüstü uygulamasından, Android uygulamasından ya da komut satırından çalışıyor. LoRa telsizi (Meshtastic ve MeshCore) ve Reticulum bağlantıları yazıldı, gerçek cihazlarda ilk saha denemesini bekliyor. İki telefon gerektiren Wi-Fi Direct ve NFC ile kart alışverişi de öyle. Son sürüm olan [0.2.0](https://github.com/kemal-arslan/carrychat/releases/tag/v0.2.0), Android uygulamasını ve Apple silikonlu Mac'ler için macOS uygulamasını içeriyor. [Yol haritasına](#yol-haritası) bak.

## Nasıl çalışır

CarryChat her mesajı elindeki en iyi bağlantıdan gönderir; bağlantı yoksa biri çıkana kadar saklar:

| Bağlantı | Menzil | Ne zaman işe yarar |
| --- | --- | --- |
| Yerel ağ | bir bina ya da bir kampüs | modem çalışıyor ama internet yok |
| Bluetooth LE | birkaç düzine metre, telefondan telefona daha uzak | hiç ağ yok |
| Wi-Fi Aware | yüz metre kadar, telefondan telefona, fotoğraf için yeterince hızlı | modem de internet de yok (destekleyen Android telefonlarda) |
| Wi-Fi Direct | yüz metre kadar, telefondan telefona, fotoğraf için yeterince hızlı | aynısı, Wi-Fi Aware'i olmayan Android telefonlarda |
| LoRa telsizi ([Meshtastic](https://meshtastic.org) ya da [MeshCore](https://meshcore.io)) | atlama başına kilometrelerce, topluluk aktarıcılarıyla daha uzak | hiç internet yok |
| [Reticulum](https://reticulum.network) | Reticulum ağının telsizleri, seri hatları ve internet bağlantıları nereye ulaşıyorsa | elde bir Reticulum ağı var |
| İnternet ([Nostr](https://nostr.com) aktarıcıları) | her yer | olağan günler ya da yoldaki bilgisayarlardan biri internete bağlandığında |
| USB bellekte dosya | biri onu nereye taşırsa | başka hiçbir şey ulaşmıyor |

- **Bir mesaj, bir telsiz paketi.** Zarf bir mesaja 104 bayt ekler (oturum başlamadan önce 67), bu yüzden 128 bayta kadar metin tek bir 233 baytlık Meshtastic paketine sığar. Daha uzun mesajlar bölünür.
- **Yerleşik Tor.** İstersen CarryChat internet aktarıcılarına Tor üzerinden ulaşır; böylece aktarıcılar IP adresini hiç görmez, ağın da CarryChat kullandığını göremez. Hiçbir zaman doğrudan bağlantıya geri dönmez: Tor çalışmadığı sürece internete çıkmaz, yerel ağı ve dosyaları kullanmaya devam eder.
- **Sakla ve ilet.** Her bilgisayar kendi giden mesajlarını saklar, başkalarınınkini de 7 güne kadar taşır ve yeni bir bağlantıyla karşılaştığında aktarır.
- **Yolda öğrenilecek bir şey yok.** Mesajı taşıyan cihazlar yalnızca her gün değişen ve yöne göre farklı olan bir etiket görür. İki anahtardan birine sahip olmadan kimden geldiğini ya da kime gittiğini anlayamazlar.
- **İleri gizlilik.** İki kişi de birbirine yazdıktan sonra anahtarlar her cevapta değişir: günlerce geç, iki kez ya da sırasız gelen mesajlara uyarlanmış bir Double Ratchet. Cihazdan sonradan alınan bir anahtar, ondan önceki mesajların hiçbirini açmaz.
- **16 kişiye kadar gruplar.** Grup mesajı, her üyenin açtığı tek bir zarftır; bu yüzden 3 kişi için de 16 kişi için de aynı yayın süresini harcar ve üyeler birbirleri için taşır. Üyelerin birbirini tanıması gerekmez. Her üye yazdığını imzalar; kimse başkasının adına yazamaz, gruptan çıkarılan bir üye de diğerleri bunu öğrendikten sonra gönderilen hiçbir şeyi okuyamaz. Tek bir telsiz paketine 97 bayta kadar metin sığar.
- **Açık kanallar.** Bir kanalın adını, varsa şifresini de bilen herkes kart alışverişi yapmadan orada okur ve yazar. Her yazar yalnızca o kanala özel bir anahtarla imzalar; böylece gönderiler ne kartına ne de başka kanallara bağlanabilir, kimse de başkasının adına gönderi yazamaz. Gönderiler bir gün yaşar ve en fazla 4 atlama gider; bir yazarı sessize alabilir, seni eklemelerini istediğinde kartını paylaşabilirsin.
- **Fotoğraflar ve sesli mesajlar.** Fotoğraf en fazla 1280 piksele ve 1 MB'a küçültülür, kendine ait bir anahtarla bir kez mühürlenir ve dilimlere bölünür; anahtar sıradan bir mesajla gider, böylece dosyalar metinle aynı korumayı alır. Dilimler yerel ağdan, internetten ve dosyalarla gider, telsizden hiç gitmez; kendi depolama kotaları vardır, mesajları hiçbir zaman dışarı itmezler.
- **Ağın denetleyebildiği teslim bilgisi.** Alıcı, yalnızca iki kişinin üretebileceği kısa bir kanıt yayımlar ve bir kopyasını taşıyan her cihaz onu siler.
- **Yalnızca kartı olan yazabilir.** Birine yazmak için onun kartı gerekir. İlk mesajın kendi kartını imzalı olarak taşır; karşı taraf parmak izinle birlikte bir istek görür, kabul eder ya da reddeder. Kartın olmayan kimse sana ulaşamaz.

Şartnamenin tamamı [docs/PROTOCOL.tr.md](docs/PROTOCOL.tr.md) dosyasında ([English](docs/PROTOCOL.md)). [docs/test-vectors.json](docs/test-vectors.json), başka bir gerçekleştirmenin kendini bayt bayt sınamasını sağlar.

## Dene

### Kurulum

[Sürümler](https://github.com/kemal-arslan/carrychat/releases) sayfasında Android ve macOS uygulamaları var:

- **Android 7 ve sonrası:** `CarryChat-0.2.0.apk` dosyasını telefona indir, aç ve o kaynaktan uygulama yüklemeye izin ver. Sonraki sürümler onu yerinde günceller.
- **Apple silikonlu macOS:** `CarryChat-0.2.0-macos-arm64.zip` dosyasını aç ve CarryChat'i Uygulamalar klasörüne taşı. Uygulama henüz Apple tarafından onaylanmadı (notarize edilmedi), bu yüzden macOS ilk açılışta onu engeller: Sistem Ayarları → **Gizlilik ve Güvenlik**'i aç ve **Yine de Aç**'a tıkla ya da `xattr -dr com.apple.quarantine /Applications/CarryChat.app` komutunu çalıştır.

Windows, Linux ve Intel Mac'ler için aşağıdaki gibi kaynaktan derlenir. Derlemek için Rust 1.85 ya da sonrası ve Node.js 20 ya da sonrası gerekir.

### Masaüstü uygulaması

```bash
cd app
npm install
npm run tauri dev
```

Kimliğini oluştur ve kartını (**Kartım**, QR kod olarak da; **Ben** sekmesinde ya da QR düğmesinde) senden haber almak istediğin kişilere ver ya da onlarınkini **Kişiler**'deki **Kişi ekle** ile ekle. Yan yana duran iki cihaz arasında ağ olmasa da olur: **Kartım**'daki **Sesle çal** kartı birkaç saniyelik tonlar olarak çalar, **Kişi ekle**'deki **Dinle** de onu doldurur. NFC'li iki Android telefon, birinde **Kartım** ötekinde **Kişi ekle** açıkken sırt sırta tutulunca da kartı aktarır. Yalnızca bir tarafın ötekinin kartına ihtiyacı vardır: ilk mesaj göndereni tanıtır, alıcı da isteği kabul eder. **Sohbetler**'deki kalem düğmesi, yazıştığın kişilerle bir grup da başlatır ya da açık bir kanala katılır. Aynı ağdaki CarryChat cihazları birbirini kendiliğinden bulur; internet varken mesajlar açık Nostr aktarıcılarından da geçer, böylece farklı ağlardaki cihazlar da birbirine ulaşır. **Ağ** sekmesi her bağlantıyı gösterir: ağın bunları engelliyorsa oradaki **Adrese bağlan** ile öteki cihazın adresini gir; **Dosyaya aktar** da teslim edilmeyi bekleyen her şeyi, birinin başka bir CarryChat cihazına taşıyıp orada içe aktarabileceği bir `.carrychat` dosyasına yazar.

**Kişiler**'deki **Yakınımdakilere görün**, onu açmış olan çevrendeki kişileri yerel ağ, Bluetooth, Wi-Fi Aware ya da Wi-Fi Direct üzerinden gösterir ve önce kart alışverişi yapmadan onlara yazmanı sağlar. Kapalıyken yakındaki kimse seni görmez, sen de kimseyi görmezsin.

**Ağ**'daki **LoRa telsizi**, bir [Meshtastic](https://meshtastic.org) ya da [MeshCore](https://meshcore.io) cihazı kullanır: telefon onunla Bluetooth üzerinden eşleşir, bilgisayar onu USB ya da ağ üzerinden kullanır; CarryChat cihazın türünü kendisi anlar. Seçildikten sonra cihazın bölgesini ya da telsiz ayarlarını ve yakın zamanda duyduğu cihazları gösterir (Meshtastic için üç saat; tekrarlayıcıları kendilerini daha seyrek duyuran MeshCore için iki gün), böylece menzilde bir ağ olup olmadığını anlarsın. Telefonda da bilgisayarda da LoRa alıcısı yoktur; bu yüzden ayrı bir cihaz gerekir. Bir MeshCore cihazında CarryChat boş bir kanal yuvasına `#carrychat` kanalını ekler.

**Ağ**'daki **Reticulum**, bir [Reticulum](https://reticulum.network) düğümüne bağlanır (örneğin aynı bilgisayarda TCP sunucu arayüzü açık bir `rnsd`) ve o ağdaki diğer CarryChat düğümlerine, kaç atlama uzakta olursa olsun ulaşır. Son altı saatte kendini duyuran CarryChat düğümlerini gösterir.

`npm run dev`, uygulamayı derlemeden arayüzü örnek verilerle bir tarayıcıda açar.

### Komut satırı

```bash
cargo install --path crates/carrychat-cli
carrychat init Ayşe
carrychat card                                  # bunu kişilerine ver
carrychat add carrychat:…                          # ya da onlarınkini ekle: bir taraf yeter
carrychat accept Deniz                          # sana önce yazan birini kabul et
carrychat send Deniz "İyiyim, Kadıköy'deyim."
carrychat send-file Deniz photo.jpg             # 1 MB'a kadar bir fotoğraf ya da sesli mesaj
carrychat save-file 3 photo.jpg                 # gelen bir dosya, kayıttaki numarasıyla
carrychat run                                   # teslim et, al ve aktar (yerel ağ ve internet)
carrychat run --tor                             # aynısı, internet yalnızca Tor üzerinden
carrychat run --bluetooth                       # yakındaki telefonlara Bluetooth ile de ulaş
carrychat run --lora /dev/cu.usbserial-0001     # ve bir Meshtastic ya da MeshCore cihazıyla LoRa (ya da tcp:host:port)
carrychat run --reticulum 127.0.0.1:4242        # ve bir rnsd TCP sunucu arayüzü üzerinden Reticulum
carrychat export messages.carrychat                # ya da her şeyi bir USB bellekte taşı
carrychat group create Mahalle Deniz Mehmet     # grubun yöneticisi sen olursun
carrychat group send Mahalle "Su dağıtımı okulun önünde."
carrychat group log Mahalle                     # ayrıca: group list, add, remove, leave
carrychat channel join "#kadıköy" --nick Ayşe   # açık bir kanal; kapalı kanal için --password
carrychat channel send kadıköy "Moda'da çadır alanı kuruldu."
carrychat channel log kadıköy                   # ayrıca: channel list, share-card, mute, nick, leave
carrychat rename "Ayşe Yılmaz"                  # kişilerin yeni adı alır
carrychat read-receipts off                     # okuduklarını kişilerine bildirmeyi bırak
carrychat erase --yes                           # kimliği, kişileri ve mesajları sil
```

Ekranı olmayan bir bilgisayarda `carrychat run` onu bir aktarıcıya çevirir: başkalarının mesajlarını okuyamadan, teslim edilene kadar taşır.

## Yol haritası

1. **Protokol v0, referans gerçekleştirme, test vektörleri** (bitti)
2. **Yerel ağ bağlantısı, mesaj deposu, masaüstü uygulaması, komut satırı, USB'ye aktarma ve içe aktarma** (bitti)
3. **Meshtastic ya da MeshCore telsiziyle LoRa bağlantısı ve Reticulum bağlantısı** (yazıldı: komut satırı ve macOS uygulaması cihazı USB ya da TCP üzerinden, Android uygulaması Bluetooth üzerinden kullanır; bölgeyi ya da telsiz ayarlarını ve duyulan cihazları gösterir. Sırada saha denemeleri var)
4. **Nostr üzerinden internet bağlantısı** (bitti). İnterneti olan bilgisayarlar, taşıdıklarını interneti olmayan bilgisayarlar için de aktarır. O bilgisayarlar adına mesaj çekme henüz açık.
5. **Android uygulaması; Bluetooth, Wi-Fi Aware ve Wi-Fi Direct bağlantıları; yakınımdakiler; sesle ve NFC ile kart** (bitti; Wi-Fi Aware, Wi-Fi Direct ve NFC iki telefon arasında denenmeyi bekliyor). iOS uygulaması, HF telsiz

## Derleme

```bash
cargo test                       # protokol, düğüm ve komut satırı
cd app && npm run tauri build    # masaüstü uygulaması yükleyicisi
```

Linux'ta masaüstü uygulaması WebKitGTK ister. [Tauri ön koşullarına](https://tauri.app/start/prerequisites/) bak.

Android uygulaması aynı projeden derlenir. [Tauri Android ön koşullarında](https://tauri.app/start/prerequisites/#android) anlatıldığı gibi kurulmuş JDK 17, NDK 27'li Android SDK ve Rust'ın Android hedefleri gerekir. Android'de OpenSSL uygulamanın içine derlenir; derlemesi, güncel NDK'larda artık bulunmayan `aarch64-linux-android-ranlib`'i arar. Bu yüzden onu NDK'nın kendi `llvm-ranlib`'ine yönlendir (Linux'ta `prebuilt/linux-x86_64` altında):

```bash
export RANLIB_aarch64_linux_android=$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-ranlib
cd app && npx tauri android build --target aarch64 --apk
```

Telefonda CarryChat ekranı kapatıldıktan sonra da, kalıcı bir bildirimle çalışmayı sürdürür; böylece uygulama görünmezken de mesajlar gelir ve başkaları için aktarılır. Telefon açıldığında ya da uygulama güncellendiğinde, açılmadan da kendiliğinden başlar. O sırada gelen mesajlar bildirim olarak duyurulur. O bildirimdeki **Turn off**, CarryChat'i yeniden açılana kadar, telefon yeniden başlasa da durdurur.

| Paket | Ne olduğu |
| --- | --- |
| [`carrychat-proto`](crates/carrychat-proto) | Kablo biçimi ve kriptografi, giriş/çıkış yok |
| [`carrychat-node`](crates/carrychat-node) | Mesaj deposu, aktarma, bağlantılar |
| [`carrychat-cli`](crates/carrychat-cli) | `carrychat` komutu |
| [`app`](app) | Masaüstü ve Android uygulaması (Tauri ve React); görünüşü [DESIGN.tr.md](DESIGN.tr.md)'ye ([English](DESIGN.md)) uyar |

## Güvenlik

Gizli anahtarların, uygulamanın veri klasöründe şifrelenmeden saklanır; o klasörü yalnızca senin kullanıcı hesabın okuyabilir. Hesabına ya da o klasörün bir kopyasına erişen herkes mesajlarını okuyabilir ve senin adına gönderebilir.

Mesajlar, iki kişi de birbirine yazdıktan sonra ileri gizliliğe sahip olur. Ondan önce gönderilen ilk mesajlarda yoktur: bir cihazın gizli anahtarını ele geçiren, kaydettiği o mesajları çözebilir. LoRa yayınları da bir telsizin yayın yaptığını ve kabaca nereden yaptığını ele verir. [docs/PROTOCOL.tr.md §10](docs/PROTOCOL.tr.md) bilinen her sınırlamayı sıralar. v1'e kadar, bulunmanın tehlikeli olduğu yerlerde CarryChat'e güvenme.

## Lisans

[MIT](LICENSE)

CarryChat özgür ve açık kaynaktır. Bir gün önemli bir mesajı taşırsa, [bana bir kahve ısmarlayabilirsin](https://buymeacoffee.com/kemalarslan).
