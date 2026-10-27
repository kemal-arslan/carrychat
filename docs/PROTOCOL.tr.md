# CarryChat protokolü v0

[English](PROTOCOL.md) · **Türkçe**. İki metin arasında fark varsa esas alınan İngilizce sürümdür.

**Durum: taslak.** v1'e kadar uyumluluğu bozan değişiklikler beklenir. Referans gerçekleme: [`crates/carrychat-proto`](../crates/carrychat-proto). [`test-vectors.json`](test-vectors.json) bu gerçeklemenin çıktısını sabitler; böylece başka gerçeklemeler kendilerini buna karşı sınayabilir.

## 1. Hedefler

- İki kişiden biri diğerinin kişi kartına sahip olur olmaz, aralarında kısa özel mesajları var olan her türlü bağlantı üzerinden iletmek: yerel ağ, LoRa telsizi, internet ya da elde taşınan bir USB bellek.
- Normal bir metin mesajı **tek bir Meshtastic paketine** (233 bayt) sığar.
- Aktarıcılar, kuryeler ve internet sunucuları içi görünmeyen baytlar taşır. Bir mesajı okuyamazlar; iki anahtardan birine sahip olmadıkça mesajın kimden geldiğini ya da kime gittiğini de anlayamazlar.
- Sakla ve ilet: bir mesajın ulaşması günler sürebilir ve birkaç türde bağlantıdan geçmesi gerekebilir.
- Kopya taşıyan herkesin doğrulayabildiği teslim alındıları; böylece mesaj ulaşır ulaşmaz kopyalar silinir.

Henüz yok: video ve LoRa üzerinden dosya.

## 2. Gösterim

- `||` birleştirmedir. Tamsayılar işaretsiz ve big-endian'dır. `u32(x)` 4 baytlık bir tamsayıdır.
- `H(x)` SHA-256'dır, `HMAC(k, x)` HMAC-SHA256'dır, `x[a..b]` ise `a` konumundan başlayıp `b` konumuna kadar olan (kendisi hariç) baytlardır.
- Zamanlar `u32` olarak unix saniyesidir. `day(t) = floor(t / 86400)`.
- `"carrychat/v0/card"` gibi sabit dizeler, sonlandırıcısı olmayan ASCII baytlarıdır.

## 3. Kimlik

Her kullanıcının birbirinden bağımsız iki anahtar çifti vardır:

- kişi kartını imzalayan bir **Ed25519** imzalama anahtarı ve
- iki taraflı anahtar anlaşması için kullanılan bir **X25519** anahtarı.

```
fingerprint = H("carrychat/v0/fingerprint" || ed25519_pk || x25519_pk)[0..16]
```

Parmak izi dolgusuz, küçük harfli RFC 4648 base32 olarak, dörderli gruplar hâlinde gösterilir (`abcd efgh ijkl mnop qrst uvwx yz`). İki kişi birbirini doğrulamak için parmak izlerini karşılaştırır.

### 3.1 Kişi kartı

| Ofset | Boyut | Alan |
| --- | --- | --- |
| 0 | 1 | sürüm, `0x00` |
| 1 | 32 | Ed25519 açık anahtarı |
| 33 | 32 | X25519 açık anahtarı |
| 65 | 1 | ad uzunluğu `N`, 0 ile 32 arası |
| 66 | N | ad, UTF-8 |
| 66+N | 64 | `"carrychat/v0/card" \|\| bytes[0..66+N]` üzerinde Ed25519 imzası |

Metin biçimi, `carrychat:` ve ardından dolgusuz base64url ile kodlanmış karttır; bir QR koduna sığacak kadar kısadır. Alıcılar imzayı doğrular (katı Ed25519 doğrulaması) ve bilinmeyen sürümleri reddeder.

v0'da ağ üzerinden kişi keşfi yoktur. Bir kişi kartını diğerine yüz yüze ya da zaten güvendikleri bir kanal üzerinden verir. Ardından ilk mesaj, gönderenin kartını öbür yöne taşır (§5.5). Kart nasıl verilirse verilsin parmak izleri karşılaştırılır.

### 3.2 Ses üzerinden kart

Yan yana duran iki cihaz, aralarında başka hiçbir ortak bağlantı olmadan bir kartı ses olarak aktarabilir: biri kartını hoparlörden çalar, diğeri mikrofondan dinler. `crates/carrychat-proto/src/sound.rs` içinde gerçeklenmiştir.

- **Tonlar.** 0'dan 95'e kadar `k` için `1875 + 46.875·k` Hz olmak üzere 96 ton, 16'şar tonluk altı kanalda: `c` kanalı `16c` ile `16c + 15` arasındaki tonları kullanır. Aralarında 46.875 Hz olan tonlar 1/46.875 s'lik bir pencere boyunca ortogonaldir; bu aralık telefon hoparlörlerine ve mikrofonlarına uygundur.
- **Semboller.** Bir sembol 100 ms sürer; başında ve sonunda 5 ms'lik yumuşak geçiş vardır. Bir veri sembolü her kanalda bir ton çalar, her biri yarım bayt taşır: kanal 0 ilk baytın üst yarım baytını, kanal 1 alt yarım baytını taşır ve bu böyle sürer; sembol başına üç bayt. Bir sembolün tüm tonları aynı ses düzeyindedir.
- **Tur.** 100 ms sessizlik; her kanalda `{0, 8}` ve `{4, 12}` tonlarını sırayla, ikişer ikişer aynı anda çalan dört sembollük bir başlangıç işareti (hiçbir veri sembolünde bu yoktur); bir başlık sembolü `0xCC || len || ~len`; tam sembole sıfırla doldurulmuş `len` bayt; 400 ms sessizlik. Bir tur en çok 255 bayt taşır. 143 baytlık bir kart yaklaşık 5.5 saniye sürer.
- **Gönderme.** Gönderen, durdurulana kadar turları tekrar tekrar çalar. Baytları denetlemek için hiçbir şey eklenmez: kart kendi imzasıyla doğrulanır (§3.1); imzası tutmayan bir kart gelirse dinleyen dinlemeyi sürdürür.
- **Dinleme.** Dinleyen, mikrofonun verdiği örnekleme hızı ne olursa olsun, Goertzel algoritmasıyla her tonu 1/46.875 s'lik pencerelerde, çeyrek pencerede bir ölçer. Art arda dört sembol boyunca işaretin tonları ortalamada her kanalın enerjisinin en az yarısını tutuyorsa işaret orada başlar; bu tür başlangıçlardan oluşan bir dizi, ortasından zamanlanır. Her sembol, ilk 30 ms'sinden (oda o sırada önceki sembolü hâlâ yankılar) sonra tamamen içinde kalan pencerelerden okunur; her kanalın en yüksek tonu o kanalın yarım baytını verir. Başlığı doğru okunamayan tur atılır.
- **Uygulamalar.** **Kartım** ekranında *Sesle çal* kartı çalar; **Kişi ekle** ekranında *Dinle*, kart duyulunca onu doldurur ve kart, yapıştırılmış bir kart gibi eklenir. Sayfa, tonları gürültü sanacak olan tarayıcı yankı engelleme, gürültü bastırma ve kazanç denetimini kapatır.

### 3.3 NFC üzerinden kart

Sırt sırta tutulan iki Android telefon bir kartı NFC üzerinden aktarır: kartını veren telefon temassız bir akıllı kart gibi yanıt verir, diğeri onu okur. Android uygulamasında gerçeklenmiştir (`CardService.kt`, `MainActivity.kt`); kartı düğüm doğrular.

- **Verme.** Kartım ekranı açıkken telefon kartını host card emulation (HCE) ile, "other" kategorisinde, `F0 43 41 52 52 59 43 48` AID'si için sunar (özel bir AID olduğunu belirten `F0`, ardından "CARRYCH"). Android bunu yalnızca ekran kilidi açıkken CarryChat'e yönlendirir; Kartım ekranı kapalıyken telefon `6A 82` yanıtını verir.
- **Okuma.** Kişi ekle ekranı açıkken telefon, okuyucu modunda NFC-A ve NFC-B etiketlerini okur ve AID için ada göre `SELECT` gönderir: `00 A4 04 00 08 || AID || 00`. Yanıt, kartın baytları (§3.1) ve ardından `90 00`'dır; bir kart en çok 162 bayttır, yani tek yanıta sığar. Düğüm kart gösterilmeden önce imzasını doğrular; ekleme, yapıştırılmış bir kartta olduğu gibidir.

## 4. Çift anahtarları

Bir kişi çifti için:

```
shared   = X25519(own_secret, peer_public)       abort if all-zero (non-contributory)
(P1, P2) = the two X25519 public keys, sorted bytewise ascending
prk      = HKDF-Extract(salt = "carrychat/v0/pair", ikm = shared)
K_enc    = HKDF-Expand(prk, "enc" || P1 || P2, 32)
K_tag    = HKDF-Expand(prk, "tag" || P1 || P2, 32)
K_ack    = HKDF-Expand(prk, "ack" || P1 || P2, 32)
K_root   = HKDF-Expand(prk, "root" || P1 || P2, 32)
K_hdr    = HKDF-Expand(prk, "hdr" || P1 || P2, 32)
```

İki kişi de aynı anahtarları türetir. Başka kimse türetemez. `K_root` ve `K_hdr` ratchet oturumlarında kullanılır (§5.6).

### 4.1 Alıcı etiketi

```
tag(recipient_pk, d) = HMAC(K_tag, recipient_pk || u32(d))[0..8]
```

`d`, mesajın oluşturulduğu gündür. Etiket, alıcıya bir zarfın hangi kişiden geldiğini, dolayısıyla hangi anahtarı denemesi gerektiğini söyler; başka hiç kimseye bir şey söylemez. Her gün değişir ve yöne göre farklıdır; bu yüzden bir gözlemci bir çiftin trafiğini günler arasında ya da bir konuşmanın iki yönü arasında ilişkilendiremez.

Alıcı, her kişinin `today − 7` ile `today + 1` arasındaki günlere ait etiketlerini önceden hesaplar: yedi gün bir zarfın en uzun ömrüdür, bir gün de saat kaymasını karşılar.

## 5. Zarf

Zarf, mühürlü tek bir mesajdır. Saklanan, taşınan ve iletilen birim odur.

| Ofset | Boyut | Alan |
| --- | --- | --- |
| 0 | 1 | sürüm, `0x00` |
| 1 | 1 | atlama sınırı |
| 2 | 4 | bitiş zamanı, `u32` |
| 6 | 8 | zarf kimliği |
| 14 | 8 | alıcı etiketi |
| 22 | 24 | nonce |
| 46 | n + 16 | şifreli metin, ardından 16 baytlık Poly1305 etiketi |

Bir zarf en çok 4096 bayttır.

### 5.1 Mühürleme

```
nonce      = 24 random bytes, never reused under the same K_enc
ack_secret = HMAC(K_ack, nonce)[0..16]
bundle_id  = H("carrychat/v0/ack" || ack_secret)[0..8]
tag        = tag(recipient_pk, day(created_at))
aad        = the 46-byte header without the hop limit byte: bytes[0..1] || bytes[2..46]
ciphertext = XChaCha20-Poly1305-Encrypt(K_enc, nonce, plaintext, aad)
```

Atlama sınırı, aktarıcılar onu azaltabilsin diye ilişkili veriye (associated data) katılmaz. Başlığın diğer tüm alanları doğrulanır.

`created_at < expires_at ≤ created_at + 604800` (7 gün). Varsayılan ömür 3 gün, varsayılan atlama sınırı 8'dir.

### 5.2 Açık metin

| Ofset | Boyut | Alan |
| --- | --- | --- |
| 0 | 1 | tür |
| 1 | 4 | oluşturulma zamanı, `u32` |
| 5 | kalanı | gövde |

| Tür | Gövde |
| --- | --- |
| `0x01` metin | UTF-8 metin |
| `0x02` tanıtım | Yalnızca tanıtımların içinde (§5.5) |
| `0x03` okundu | Alıcının zarflarından gönderenin okuduklarına ait en çok 64 zarf kimliği (her biri 8 bayt). İsteğe bağlıdır: kullanıcı okundu bilgisini kapatabilir. |
| `0x04` kart | Ad değişikliğinden sonra gönderenin kişi kartı (§3.1). Alıcı kartı yalnızca iki anahtar da elindeki karttakilerle aynıysa kabul eder. |
| `0x05` oturum | Boş. Bir ratchet oturumu başlatır (§5.6) ve gösterilmez. |
| `0x06` grup | Grubun yöneticisinden gelen bir grup durumu (§5.7). |
| `0x07` gönderen anahtarı | Bir üyenin bir grup için gönderen zinciri (§5.7). |
| `0x08` ayrılma | Bir grup kimliği (16 bayt): gönderen o gruptan ayrılır (§5.7). |
| `0x09` dosya | Bir dosya teklifi (§5.9): dosyanın ne olduğu ve dilimlerinin anahtarı. Gruplarda ve kanallarda da kullanılır. |

Diğer tüm değerler ayrılmıştır. Bilinmeyen türde bir zarfı açan alıcı gövdeyi yok sayar ama yine de bir `tombstone` (zarfın teslim edildiğini, işinin bittiğini bildiren kayıt) üretir; böylece zarf dolaşmayı bırakır.

### 5.3 Açma

1. Kişiyi ve günü bulmak için etiket, önceden hesaplanmış etiketler arasında aranır.
2. O kişinin `K_enc` anahtarıyla şifre çözülür. Şifre çözme başarısız olursa zarf bize ait değildir ya da bozuktur.
3. `tag == tag(own_pk, day(created_at))` olduğu, §5.1'deki ömür kuralının sağlandığı ve `bundle_id`'nin nonce'tan türetilen değer olduğu denetlenir. Denetimlerden biri başarısız olursa zarf reddedilir.
4. Bir `tombstone` üretilir (§6).

### 5.4 Boyut bütçesi

Ek yük: 46 (başlık) + 5 (açık metin başlığı) + 16 (Poly1305 etiketi) = **67 bayt**. 1 baytlık çerçeve türüyle (§7) birlikte 233 baytlık tek bir Meshtastic paketinde gövde en çok **165 bayt** olabilir: 165 ASCII karakter; UTF-8'de her biri iki bayt tutan ç, ğ, ı, ö, ş ve ü gibi harfler içeren metinde daha az. Bir ratchet mesajı (§5.6) 37 baytlık başlığını ekler: 104 bayt ek yük ve en çok 128 baytlık gövde.

### 5.5 Tanıtımlar

Tanıtım, gönderenin kartına henüz sahip olmayan birine gönderilen ilk mesajdır. Yalnızca gönderenin alıcının kartına sahip olması gerekir. §5'teki başlığa sahip bir zarftır, şu farklarla: alıcı etiketi, alıcının oluşturulduğu günkü posta kutusu etiketidir (§9.3), böylece onu internet aktarıcılarına taşıyan bir düğüm, alıcının kim olduğunu bilmeden onu alıcının baktığı yere yayımlar; şifreli metin alanı da tek kullanımlık bir açık anahtarla başlar:

```
e          = a fresh X25519 key pair, used once
shared     = X25519(e_secret, recipient_pk)                abort if all-zero
prk        = HKDF-Extract(salt = "carrychat/v0/intro", ikm = shared)
K_enc      = HKDF-Expand(prk, "enc" || e_pk || recipient_pk, 32)
K_ack      = HKDF-Expand(prk, "ack" || e_pk || recipient_pk, 32)
ack_secret = HMAC(K_ack, nonce)[0..16]
bundle_id  = H("carrychat/v0/ack" || ack_secret)[0..8]
signature  = Ed25519-Sign(sender's signing key, "carrychat/v0/intro-signature"
               || e_pk || recipient_pk || nonce || u32(created_at) || text)
plaintext  = 0x02 || u32(created_at) || u8(len(card)) || card || signature || text
ciphertext = e_pk || XChaCha20-Poly1305-Encrypt(K_enc, nonce, plaintext, aad)
```

`card` gönderenin kişi kartıdır (§3.1), `aad` ise §5.1'dekidir.

**Alma.** Düğüm, etiketi hiçbir kişisiyle eşleşmeyen her zarfı tanıtım olarak dener: `X25519(own_secret, e_pk)` değerini hesaplar, anahtarları türetir ve şifreyi çözer. Bu başarılı olursa önce kartın kendi imzasını, sonra kartın Ed25519 anahtarıyla `signature`'ı, ardından ömür kuralını ve zarf kimliğini doğrular. Kart kime ait olduğunu kanıtlar; imza ise gönderenin o kartın anahtarına sahip olduğunu kanıtlar ve mesajı bu alıcıya ve bu zarfa bağlar, böylece mesaj başka birine yeniden oynatılamaz. Bir tanıtımı açan düğüm, her zarfta olduğu gibi bir `tombstone` üretir (§6).

**İstekler.** Gönderen henüz bir kişi değilse mesaj bir *istek* olur: uygulama mesajı gönderenin adı ve parmak iziyle gösterir, kullanıcı kabul eder ya da reddeder. Kabul etmek göndereni kişi yapar. Reddetmek isteği siler ve gönderenin anahtarını hatırlar: ondan gelen sonraki tanıtımlar atılır, ama dolaşmayı bıraksınlar diye yine de `tombstone` ile kapatılır. İstek yanıtlanmadan önce istek sahibinden gelen çift zarfları (§5.1) istekle birlikte saklanır.

**Gönderme.** Düğüm, kişinin kendi kartına sahip olduğunu öğrenene kadar o kişiye tanıtım gönderir: o kişiye gönderdiği zarflardan biri için bir `tombstone` geri gelmiştir ya da o kişiden bir zarf ulaşmıştır. Bundan sonra çift zarfları gönderir.

Kısa bir tanıtım iki Meshtastic paketi tutar: ek yükü 164 bayt artı karttır; kart da 130 bayt artı gönderenin adıdır.

### 5.6 Ratchet oturumları

`K_enc` ile mühürlenen çift zarflarında (§5.1) ileri gizlilik yoktur: bir cihazın X25519 gizli anahtarını sonradan ele geçiren, kaydedilmiş her zarfı açabilir. Ratchet oturumu bunu, zarfların günler sonra, sırasız, iki kez ya da birkaç yoldan ulaştığı bir ağa uyarlanmış Double Ratchet (Signal) ile giderir.

**Roller.** İki kişiden X25519 açık anahtarı sıralamada önce gelen *başlatıcı*, diğeri *yanıtlayıcı*dır. Oturumu yalnızca başlatıcı başlatır; böylece ikisi asla aynı anda iki oturum başlatamaz. İkisi de aynı durumdan başlar: kök anahtar `K_root`, kendi ratchet anahtar çifti = kendi X25519 anahtar çifti, karşı tarafın ratchet anahtarı = başlatıcı için karşı tarafın X25519 anahtarı, yanıtlayıcı için yok; zincir yok.

**Fonksiyonlar.**

```
KDF_RK(rk, dh) = HKDF-SHA256(salt = rk, ikm = dh, info = "carrychat/v1/ratchet", 64)  → (rk', ck)
KDF_CK(ck)     = (HMAC(ck, 0x02), HMAC(ck, 0x01))                                  → (ck', mk)
```

**Gönderme.** Gönderenin gönderme zinciri yoksa ve gönderen başlatıcıysa oturumu başlatır: yeni bir ratchet anahtar çifti `DHs`, ardından `(RK, CKs) = KDF_RK(RK, X25519(DHs, DHr))`. Sonra `(CKs, mk) = KDF_CK(CKs)` hesaplanır ve zarf aşağıdaki gibi mühürlenir; `Ns` geçerli gönderme zincirindeki mesajları sayar, `PN` önceki zincirin uzunluğudur.

```
header     = 0x01 || DHs_public (32) || u16(PN) || u16(Ns)               37 bytes
header_ct  = header XOR XChaCha20(K_hdr, nonce)[0..37]                    keystream from block 0
ciphertext = header_ct || XChaCha20-Poly1305-Encrypt(mk, nonce, plaintext, aad || header_ct)
```

`aad` §5.1'dekidir; dış zarf, etiketi, zarf kimliği ve `tombstone`'u tam olarak §5'teki gibidir. Başlığın kendine ait bir etiketi yoktur: başlığı değiştirmek mesaj anahtarını değiştirir ve AEAD başarısız olur. Başlığı şifrelemek, aksi hâlde bir konuşmanın zarflarını birbirine bağlayacak olan ratchet anahtarlarını çift anahtarlarına sahip olmayan herkesten gizler. Henüz gönderme zinciri olmayan bir yanıtlayıcı, §5.1'deki gibi `K_enc` ile mühürler.

**Alma.** Alıcı ilk 37 baytın şifresini anahtar akışıyla çözer; ilk bayt `0x01` ise zarfı ratchet mesajı olarak, değilse §5.1'deki gibi açar. Bir ratchet mesajı için, varsa `(DHs_public, Ns)` için saklanmış atlanmış anahtarı kullanır. Yoksa ve `DHs_public` karşı tarafın ratchet anahtarından farklıysa, geçerli alma zincirinin geri kalanını `PN`'ye kadar türetir, ardından bir DH ratchet adımı atar: `PN = Ns`, `Ns = Nr = 0`, `DHr = DHs_public`, `(RK, CKr) = KDF_RK(RK, X25519(own, DHr))`, yeni bir kendi ratchet anahtar çifti, `(RK, CKs) = KDF_RK(RK, X25519(own, DHr))`. Sonra zinciri `Ns`'ye kadar türetir ve mesaj anahtarını elde eder. Bir zinciri ileri doğru türettiği her seferde ne kadar ileri olursa olsun türetir ve geçtiği son 1000 mesajın anahtarlarını atlanmış anahtar olarak saklar: uzun bir sessizlikten sonra eski mesajların süresi yolda dolmuştur ve daha ileri gitmeyi reddetmek oturumu kalıcı olarak kilitlerdi. Her değişiklik yalnızca AEAD başarılı olursa saklanır.

**Başlatma.** Başlatıcı, kişisinden gelen bir `K_enc` zarfını açtığında gönderme zinciri yoksa bir oturum mesajıyla (tür `0x05`) yanıt verir; böylece yanıtlayıcı ratchet mesajlarına geçebilir.

**Temizlik.** Mesaj anahtarları kullanılır kullanılmaz silinir. Atlanmış anahtarlar kullanıldıklarında ve 8 gün sonra silinir. Depo, silinen verinin üzerine yazar (SQLite `secure_delete`). Bir zincir 65535 mesaja izin verir; karşı taraftan haber almadan bu sayıya ulaşan gönderen `K_enc`'e geri döner.

### 5.7 Gruplar

Bir grup mesajı, üye sayısı ne olursa olsun her üyenin açabildiği tek bir zarftır. Her üyenin kendine ait bir *gönderen zinciri* vardır; bunu çift zarflarıyla (§5.1, §5.6) diğer üyelere verir ve grup mesajlarını onunla mühürleyip imzalar.

**Durum.** Grubu oluşturan üye grubun yöneticisidir. Grup durumu şöyledir:

```
group_id (16) || u32(version) || u8(len(name)) || name || u8(n) || (u8(len(card)) || card) × n
```

`group_id` 16 rastgele bayttır, `name` en çok 32 baytlık UTF-8'dir; `n` kart (§3.1) ise yönetici dahil üyelerdir, en çok 16 tane. Yönetici her değişiklikte `version` değerini artırır ve yeni durumu, az önce çıkardığı üyeler dahil her üyeye `0x06` türünde bir çift zarfıyla gönderir. Düğüm, bilmediği bir grubun durumunu yalnızca kabul ettiği bir kişiden alır ve o kişi grubun yöneticisi olur; bundan sonra durumu yalnızca o yöneticiden ve yalnızca daha yüksek bir sürümle alır. Üye listesinde olmayan düğüm gruptan ayrılmış demektir.

**Eşler.** Durum her üyeye diğerlerinin kartlarını verir. Kişi olmayan bir üye *grup eşi* olur: düğüm onunla çift anahtarlarını ve etiketleri türetir (§4), onunla çift zarfı alışverişi yapar ve onun kendi kartına sahip olduğunu varsayar, ama ondan gelen doğrudan mesajları göstermez.

**Gönderen zincirleri.** Bir üye, bir gruba katıldığında bir gönderen zinciri başlatır; bir üyenin ayrıldığını ya da çıkarıldığını her gördüğünde de yeniden başlatır:

```
sender_key = 32 random bytes
CK         = 32 random bytes, at iteration 0
(sk, pk)   = a fresh Ed25519 key pair
```

Zincirin o anki hâlini diğer her üyeye `0x07` türünde bir çift zarfıyla gönderir:

```
group_id (16) || sender_key (32) || u32(iteration) || CK (32) || pk (32)
```

Bir üye katıldığında diğer her üye ona kendi geçerli zincirini aynı şekilde gönderir; böylece yeni üye daha önce gönderilmiş hiçbir şeyi okuyamaz. Düğüm aldığı bir zinciri, gruptan ayrılana kadar ya da aynı üye daha yeni bir zincir gönderdikten 8 gün sonrasına kadar saklar.

**Mühürleme.** Bir zincirin `i` numaralı mesajı §5'teki zarf başlığıyla mühürlenir:

```
prk        = HKDF-Extract(salt = "carrychat/v1/group", ikm = sender_key)
K_gtag     = HKDF-Expand(prk, "tag", 32)
K_ghdr     = HKDF-Expand(prk, "hdr", 32)
(CK', mk)  = KDF_CK(CK)                                 as in §5.6; CK' is at iteration i + 1
tag        = HMAC(K_gtag, u32(day(created_at)))[0..8]
bundle_id  = H("carrychat/v1/group" || nonce)[0..8]
header_ct  = u32(i) XOR XChaCha20(K_ghdr, nonce)[0..4]    keystream from block 0
sealed     = XChaCha20-Poly1305-Encrypt(mk, nonce, plaintext, aad || header_ct)
signature  = Ed25519-Sign(sk, "carrychat/v1/group-signature" || aad || header_ct || sealed)
ciphertext = header_ct || sealed || signature
```

`aad` §5.1'dekidir, açık metin de §5.2'dekidir. Sürüm 1 gruplarda metin (tür `0x01`) ve dosya (tür `0x09`, §5.9) gönderir. Etiket her gün değişir ve zincire göre farklıdır; bu yüzden taşıyıcılar bir zarfın hangi gruba ait olduğunu da, hangi üyenin gönderdiğini de anlayamaz.

**Alma.** Üye, elindeki her zincirin etiketlerini etiket penceresi (§4.1) boyunca önceden hesaplar. Etiketi eşleşen bir zarf için imzayı zincirin `pk`'siyle doğrular (katı Ed25519), başlığın şifresini çözer ve `i` yinelemesinin mesaj anahtarını alır: ya saklanmış bir atlanmış anahtar ya da en çok 100 000 yineleme ileri türetilmiş zincir; bu sırada geçtiği son 1000 anahtarı saklar. Sonra şifreyi çözer ve etiketi `day(created_at)`'e, §5.1'deki ömür kuralına ve zarf kimliğine göre denetler. Değişiklikler yalnızca her adım başarılı olursa saklanır ve her anahtar tek bir mesaj açar. Zarf yalnızca zincir, düğümün o anda üye saydığı birine aitse kabul edilir. Düğüm yeni bir zincir ya da üye öğrendiğinde, yeni bir kişi için yaptığı gibi (§8), taşıdığı zarfları yeniden açar.

**Teslim.** Grup zarfının `tombstone`'u yoktur: bir üyenin alındısı, zarfın diğerlerine ulaşmasını durdurmamalıdır. Zarf süresi dolana kadar dolaşır ve üyeler onu birbirleri için taşır.

**Ayrılma.** Bir üye, yöneticiye grup kimliğini içeren `0x08` türünde bir çift zarfı göndererek ayrılır; yönetici de onu çıkarır. Yönetici ayrılamaz ama diğer tüm üyeleri çıkarabilir. Çıkarılan üye, çıkarılmasından sonra başlatılan zincirlerin hiçbirini almaz. Ancak bir üye yeni durumu görene kadar eski zinciriyle yazmayı sürdürür ve çıkarılan üye bunu hâlâ okuyabilir.

**Boyut.** Bir grup zarfı 46 (başlık) + 4 (yineleme) + 5 (açık metin başlığı) + 16 (Poly1305 etiketi) + 64 (imza) = **135 bayt** ekler; böylece tek bir Meshtastic paketine en çok **97 bayt** metin sığar.

### 5.8 Herkese açık kanallar

Herkese açık kanal, adını bilen herkesin kart ya da kişi gerekmeden okuyup yazabildiği bir konuşmadır. Bir kanalın parolası da olabilir: o zaman yalnızca ikisini de bilenler okuyup yazabilir.

**Adlar.** Kanal adının baştaki ve sondaki boşlukları kırpılır, baştaki `#` atılır ve ad Unicode varsayılan büyük/küçük harf eşlemesiyle (yerel ayar kuralları olmadan) küçük harfe çevrilir; ardından `i`'den sonra gelen birleşik üst nokta (U+0307) atılır, böylece `İ`, `i` olur. Sonuç harflerden, rakamlardan (Unicode Alphabetic ve Numeric), `-` ve `_` karakterlerinden oluşan 1 ile 32 bayt arası UTF-8 olmalıdır. `#Mahalle` ve `mahalle` aynı kanaldır.

**Anahtarlar.**

```
prk     = HKDF-Extract(salt = "carrychat/v1/channel", ikm = name || 0x00 || password)
K_ch    = HKDF-Expand(prk, "enc", 32)
K_chtag = HKDF-Expand(prk, "tag", 32)
K_chid  = HKDF-Expand(prk, "id", 32)
tag(d)  = HMAC(K_chtag, u32(d))[0..8]
```

`password` UTF-8'dir; parolası olmayan kanal için boştur. Adı ve parolayı bilen herkes aynı anahtarları türetir; böylece etiket üyelere hangi zarfların kanala ait olduğunu söyler, başka kimseye bir şey söylemez. Etiket her gün değişir.

**Yazarlar.** Her yazar, o kanala özgü kendi Ed25519 anahtarıyla imzalar; böylece farklı kanallardaki gönderileri birbirine ya da kartına bağlanamaz. Referans gerçekleme bu anahtarı `seed = HMAC(own Ed25519 secret, "carrychat/v1/channel-key" || K_chid)` olarak türetir. Her gönderi ayrıca serbestçe seçilen, en çok 20 baytlık bir takma ad taşır; bu yüzden okuyucular yazarları ada göre değil anahtara göre ayırt eder.

**Mühürleme.**

```
plaintext  = kind || u32(created_at) || pk (32) || u8(len(nick)) || nick || body
bundle_id  = H("carrychat/v1/channel" || nonce)[0..8]
sealed     = XChaCha20-Poly1305-Encrypt(K_ch, nonce, plaintext, aad)
signature  = Ed25519-Sign(sk, "carrychat/v1/channel-signature" || aad || sealed)
ciphertext = sealed || signature
```

`aad` §5.1'dekidir, etiket ise `tag(day(created_at))`'dir. Tür; metin (`0x01`, gövde UTF-8), kart (`0x04`, gövde yazarın kişi kartıdır; okuyucular onu ekleyebilsin diye paylaşılır) ya da dosyadır (`0x09`, bir teklif, §5.9). Bir kanal zarfı en çok bir gün yaşar (`expires_at ≤ created_at + 86400`) ve 4 atlama sınırıyla başlar; böylece yoğun bir kanal ağı doldurmaz.

**Alma.** Üye, kanallarının etiketlerini etiket penceresi (§4.1) boyunca önceden hesaplar. Etiketi eşleşen bir zarfın şifresini çözer, imzayı içindeki `pk` ile doğrular (katı Ed25519) ve etiketi `day(created_at)`'e, kanal ömrüne ve zarf kimliğine göre denetler. Grup zarfı gibi kanal zarfının da `tombstone`'u yoktur ve üyeler onu birbirleri için taşır. Bir üye bir yazarın anahtarını susturabilir: bu durumda o yazarın gönderilerini ne gösterir ne de iletir. Referans gerçekleme ayrıca bir kanala dakikada 10'dan fazla gönderi göndermeyi reddeder.

**Boyut.** Bir kanal zarfı 46 (başlık) + 5 (açık metin başlığı) + 33 (anahtar ve takma ad uzunluğu) + 16 (Poly1305 etiketi) + 64 (imza) = **164 bayt** artı takma ad ekler; böylece 8 baytlık bir takma adla tek bir Meshtastic paketine en çok **60 bayt** metin sığar.

### 5.9 Dosyalar

Bir fotoğraf ya da sesli mesaj bir zarf için fazla büyüktür. Kendine ait bir anahtarla bir kez mühürlenmiş dilimler hâlinde yol alır; anahtar ise bir kişiye, gruba ya da kanala giden `0x09` türünde sıradan bir mesajla gider. Bir grup ya da kanal, tüm üyeleri için tek bir dilim takımı alır.

**Teklif.** Bir dosya mesajının gövdesi şöyledir:

```
u8(len(mime)) || mime || u32(size) || K_f (32) || H(file) (32)
```

`mime`, `image/jpeg` ya da `audio/mp4` gibi medya türüdür; `size` dosyanın uzunluğudur (en çok 1 MiB); `K_f` ise bu dosya için seçilmiş 32 rastgele bayttır.

**Dilimler.** Dosya 4000 baytlık dilimlere bölünür, sonuncusu daha kısadır; yani dosyanın `n = ceil(size / 4000)` dilimi vardır. `i` numaralı dilim, §5'teki başlığa sahip bir zarftır:

```
prk        = HKDF-Extract(salt = "carrychat/v1/file", ikm = K_f)
K_fenc     = HKDF-Expand(prk, "enc", 32)
K_ftag     = HKDF-Expand(prk, "tag", 32)
K_fack     = HKDF-Expand(prk, "ack", 32)
tag        = HMAC(K_ftag, u32(i))[0..8]
ack_secret = HMAC(K_fack, nonce)[0..16]
bundle_id  = H("carrychat/v0/ack" || ack_secret)[0..8]
ciphertext = XChaCha20-Poly1305-Encrypt(K_fenc, nonce, file[4000·i .. 4000·(i+1)], aad)
```

`aad` §5.1'dekidir; böylece etiket dilimi kendi yerine bağlar. Dilimlerin süresi, onları teklif eden mesajla birlikte dolar ve dilimler o mesajın atlama sınırıyla başlar. Dilim çerçevelerinde (§7) yol alırlar ve bu çerçeveler asla LoRa üzerinden gitmez: 1 MiB'lik bir dosya saatlerce yayın süresi tutardı.

**Alma.** Bir teklifi açan düğüm, dilimlerin etiketlerini hesaplar ve bunlarla eşleşen dilim zarflarını, zaten taşımakta olduğu dilimler dahil, herhangi bir bağlantıdan ve herhangi bir sırayla alır. `n` dilimin hepsine sahip olunca onları birleştirir, uzunluğu ve `H(file)` değerini denetler; denetimi geçemeyen dosya atılır. İki kişi arasındaki bir konuşmada alıcı, aldığı her dilimin `tombstone`'unu yayımlar (§6); böylece taşıyıcılar dilimi atar. Grupta ya da kanalda bunu yapmaz; üyeler, grup zarflarında olduğu gibi dilimleri birbirleri için taşır.

**Taşıma.** Düğümler başkaları için taşıdıkları dilimleri, mesajlarınkinden ayrı, kendine ait bir kota altında tutar; böylece dosyalar mesajları asla dışarı itmez. İnternet üzerinden (§9.3) düğüm dilimlerini her aktarıcıya ölçülü bir hızla yayımlar; böylece bir fotoğraf aktarıcıların hız sınırları içinde kalır.

## 6. `tombstone`

Bir `tombstone` teslimi kanıtlar. Alıcı bir zarfı açtıktan sonra `(bundle_id, ack_secret)` değerini yayımlar.

Zarfı tutan herkes `H("carrychat/v0/ack" || ack_secret)[0..8] == bundle_id` eşitliğini denetler, sonra zarfı siler ve iletmeyi bırakır. Gönderen aynı denetimi yapar ve mesajı teslim edildi olarak işaretler. `K_ack`'i yalnızca iki kişi bilir; bu yüzden gerçek bir zarf için `ack_secret` üretebilen de yalnızca onlardır. Birini sahtelemek, 64 bite kırpılmış bir özetin ön görüntüsünü bulmak demektir.

- Düğüm bir `tombstone`'u yalnızca tuttuğu ya da gördüğü bir zarf kimliği için iletir. Herkes uydurduğu bir kimlik için kendi içinde tutarlı bir `tombstone` oluşturabilir; bu yüzden bilinmeyen kimliklere ait `tombstone`'lar atılır.
- Düğüm, `tombstone`'u gelmiş kimlikleri zarfın süresinin dolacağı zamana kadar hatırlar; böylece geç gelen bir kopya yeniden kabul edilmez.

Bir `tombstone`, bir zarfın yaklaşık bu zamanda ve bu yerde teslim edildiğini açığa çıkarır. Bunu istemeyen alıcı `tombstone`'u geciktirebilir ya da hiç göndermeyebilir; bu durumda zarf süresi dolana kadar dolaşır.

## 7. Çerçeveler

Aktarımlar çerçeve taşır. MTU'suna bütün bir zarf sığan bir aktarım (yerel ağ, internet, dosya) asla parçalama yapmaz.

| Tür | Düzen |
| --- | --- |
| `0x01` zarf | `type \|\| bundle` |
| `0x02` parça | `type \|\| bundle_id (8) \|\| index (1) \|\| count (1) \|\| chunk` |
| `0x03` `tombstone` | `type \|\| hop_limit (1) \|\| bundle_id (8) \|\| ack_secret (16)`, 26 bayt |
| `0x04` dilim | `type \|\| bundle`, bir dosya dilimi (§5.9). Asla LoRa üzerinden gönderilmez. |

`1 + len(bundle) > mtu` ise zarfın baytları `mtu − 11` baytlık dilimlere bölünür; dilim sayısı 2 ile 32 arasındadır. Alıcılar bir zaman aşımıyla (varsayılan 6 saat: en büyük zarf LoRa üzerinden Meshtastic'in en yavaş ayarında, §9.2'deki yayın süresi payı içinde beş saatten uzun sürer) ve eksik zarf sayısına bir üst sınırla zarf kimliğine göre yeniden birleştirir, sonra birleşen zarfın parçalarıyla aynı kimliği taşıdığını denetler. Aktarıcılar iletmeden önce yeniden birleştirir, çünkü atlama sınırına ve bitiş zamanına ihtiyaç duyarlar.

`0x10` ile `0x1f` arasındaki çerçeve türleri, aktarıma özgü denetim trafiği için ayrılmıştır (§9.1).

## 8. İletme

Her düğüm tek bir depo tutar: kendi giden kutusu ile başkaları için taşıdığı zarflar. Bir çerçevenin işlenmesi atomiktir: herhangi bir kısmı başarısız olursa (örneğin depoya yazma), hiçbir kısmı saklanmaz; böylece saklanmamış bir mesaj için asla `tombstone` çıkmaz.

- Yinelenen zarflar zarf kimliğine göre atılır.
- Süresi dolan zarflar silinir. Süresinin şu andan itibaren 8 günden daha sonra dolacağını öne süren zarflar reddedilir.
- İletme atlama sınırını bir azaltır. Atlama sınırı 0 olan zarf saklanır ama iletilmez.
- Başkaları için taşınan zarfların bir kotası vardır. Kota dolduğunda önce süresinin dolmasına en az kalan zarf çıkarılır.
- Bir zarf geldiği bağlantı üzerinden geri gönderilmez. `tombstone` ise gönderilir, çünkü öbür uçtaki düğüm onu bekleyen gönderen olabilir.
- Bir kişi ekleyen düğüm, zaten taşımakta olduğu ve o kişiden geldiği anlaşılan zarfları açar.
- **LoRa'da bölünmüş ufuk (split horizon).** LoRa üzerinden gelen bir zarf, aynı düğüm tarafından yeniden LoRa üzerinden gönderilmez, çünkü Meshtastic ya da MeshCore ağı onu zaten her yöne yayar (flood). Diğer bağlantılara köprülenebilir. Başka herhangi bir bağlantıdan gelen zarflar, yayın süresi bütçesi içinde LoRa üzerinden gönderilebilir; önce düğümün kendi zarfları ve `tombstone`'ları gider.

## 9. Aktarımlar (taslak)

Bu eşlemeler her aktarım hayata geçirildiğinde kesinleşir (README'deki yol haritasına bakın).

### 9.1 Yerel ağ

`crates/carrychat-node/src/lan.rs` içinde gerçeklenmiştir.

**Keşif.** Düğüm her 3 saniyede bir `239.255.74.74` çok noktaya yayın (multicast) grubuna, `47474` portuna 16 baytlık bir işaret paketi (beacon) gönderir:

| Ofset | Boyut | Alan |
| --- | --- | --- |
| 0 | 6 | `"CRCH" \|\| 0x00 \|\| 0x01` (sihirli değer, sürüm, işaret paketi) |
| 6 | 8 | oturum kimliği, her çalıştırmada rastgele |
| 14 | 2 | TCP portu, `u16` |

İşaret paketi hiçbir kimlik taşımaz. Bağlı olmadığı bir düğümün işaret paketini duyduğunda, oturum kimliği küçük olan düğüm bağlantı kurar. Kendi işaret paketleri karşıya ulaşmıyor olabilir diye, diğer düğüm de bağlantı kurulmadan geçen üç işaret paketinden sonra bağlantı kurar.

**Eşitleme.** `crates/carrychat-node/src/sync.rs` içinde gerçeklenmiştir; Bluetooth LE (§9.5) ve Wi-Fi Aware (§9.6) üzerinden de aynıdır. TCP üzerinden her çerçevenin önüne `u16` uzunluğu eklenir. §7'deki çerçevelerin yanında dört denetim çerçevesi vardır:

| Tür | Düzen |
| --- | --- |
| `0x10` HAVE | `type \|\| count (u16) \|\| bundle_id × count` |
| `0x11` WANT | `type \|\| count (u16) \|\| bundle_id × count` |
| `0x12` HELLO | `type \|\| session id (8)` |
| `0x13` NEARBY | `type \|\| kind (1) \|\| card` (§9.7) |

İki taraf da önce HELLO gönderir. Aynı iki düğüm arasında, hangi yoldan kurulursa kurulsun, ikinci bir bağlantı kapatılır. Sonra her taraf, iletebileceği zarfların kimliklerini içeren bir HAVE ve bildiği her `tombstone`'u gönderir. Karşı taraf hiç görmediği kimlikler için WANT ile yanıt verir ve o zarfları alır. Bundan sonra yeni zarflar ve `tombstone`'lar ortaya çıktıkça gönderilir. Her 15 saniyede bir gönderilen boş bir HAVE bağlantıyı canlı tutar; 45 saniye sessiz kalan bağlantı kapatılır.

### 9.2 Meshtastic ya da MeshCore ile LoRa

`crates/carrychat-node/src/lora.rs` içinde gerçeklenmiştir; cihazların protokolleri `lora/meshtastic.rs` ve `lora/meshcore.rs` içindedir. Masaüstü uygulamaları ve komut satırı cihaza USB seri ya da TCP üzerinden, telefonlar Bluetooth LE üzerinden ulaşır.

- **Hangi cihaz.** USB seri ya da TCP üzerinden düğüm önce MeshCore uygulamalarının yaptığı gibi sorar (`CMD_APP_START`), 2 saniye sonra yeniden sorar ve 4 saniye içinde yanıt vermeyen cihazı Meshtastic cihazı sayar. Önce MeshCore denenir: TCP'deki bir MeshCore cihazı herhangi üç baytı çerçeve başlığı olarak okur ve Meshtastic'in çerçeveleri gelince kendi çerçevelerinin izini kaybeder; Meshtastic cihazı ise kendisine ait olmayan baytları atlar. Bluetooth LE üzerinden, cihazın sunduğu servis hangisi olduğunu söyler.
- **Meshtastic cihazı.** Düğüm bir Meshtastic cihazıyla Meshtastic istemci API'si üzerinden konuşur: 115200 baud'da USB seri, TCP (ağdaki bir cihaz ya da meshtasticd, 4403 portu) ya da Bluetooth LE üzerinden. Seri ve TCP, API'nin akış çerçevelemesini taşır: `0x94 0xC3 || u16 length || protobuf`. Bluetooth LE üzerinden, Meshtastic'in kendi telefon uygulamalarının yaptığı gibi, her `ToRadio` mesajı ToRadio karakteristiğine yazılır; FromNum her bildirim gönderdiğinde `FromRadio` mesajları FromRadio'dan boş okunana kadar tek tek okunur. Bunu yapan platform kodu mesajları seri porttaki gibi çerçeveler, böylece düğüm bunun üzerinde aynı oturumu çalıştırır. Cihaz önce eşleştirilmiş olmalıdır. Seri porttaki ilk mesajdan önce düğüm 32 bayt `0xC3` gönderir; bu, günlüğünü yazdıran bir cihazı API'ye geçirir. Sonra yapılandırmayı ister (`want_config_id`), radyonun modem ayarlarını ve atlama sınırını bundan okur ve `config_complete_id` gelince başlar. Her 5 dakikada bir gönderilen bir yaşam sinyali (heartbeat) bağlantıyı açık tutar. Bağlantı sona erdiğinde düğüm 10 saniye sonra yeniden bağlanır.
- **Cihazı bekleyenler.** Cihaz hangi türden olursa olsun, bağlandığında düğüm hâlâ alındı bekleyen kendi zarflarını ve kendisine teslim edilen son 8 zarfın `tombstone`'larını kuyruğa alır; bağlantı başladığından beri bunları zaten kuyruğa aldıysa almaz. Yoksa cihaz bağlanmadan önce yazılan mesajlar başka bir bağlantıyı beklerdi.
- **Meshtastic cihazının duydukları.** Yapılandırma ayrıca cihazın bildiği cihazları (`NodeInfo`) ve cihazın bölgesini listeler. Düğüm bunların adlarını; her birinin en son ne zaman, ne güçte ve kaç atlama üzerinden duyulduğunu (cihazın duyduğu her paketle güncellenir) ve CarryChat'in LoRa üzerinden en son ne zaman geldiğini tutar. Uygulamalar son üç saatte duyulan cihazları gösterir; bu da bir ağın erişim alanında olup olmadığını söyler. Bölgesi `UNSET` olan cihaz yayın yapmaz ve uygulamalar bunu belirtir. Cihaz bulmak için fazladan hiçbir şey gönderilmez: dinlemek bedavadır, yayın süresi değildir.
- **Meshtastic paketleri.** Her çerçeve, birincil kanalda cihazın atlama sınırıyla yayınlanan bir Meshtastic `Data` paketinin yüküdür; MTU 233 bayttır (`DATA_PAYLOAD_LEN`). Bundan uzun zarflar parçalar hâlinde gider (§7). Geliştirme sürecinde `PRIVATE_APP` (256) portnum'u kullanılır. Biçim oturunca özel bir portnum istenecektir. LoRa üzerinden yalnızca zarflar ve `tombstone`'lar gider, asla dosya dilimleri gitmez.
- **Meshtastic kanalı.** Birincil kanal varsayılan olarak herkese açık kanaldır; böylece aynı bölge ve modem ön ayarına sahip topluluk cihazları CarryChat paketlerini diğer paketler gibi aktarır. Başka adlı bir birincil kanal başka bir frekansa geçer ve orada bu cihazlar onu duymaz. CarryChat zarfları zaten uçtan uca şifrelidir; bu yüzden herkese açık kanalın ortak anahtarı hiçbir şey kaybettirmez.
- **MeshCore cihazı.** Düğüm MeshCore'un companion protokolünü konuşur: 115200 baud'da USB seri, TCP (5000 portu) ya da Bluetooth LE üzerinden. Seri ve TCP her mesajı `<` (cihaza) ya da `>` (cihazdan) `|| u16 length (little-endian) || body` olarak çerçeveler. Bluetooth LE üzerinden her gövde, Nordic UART servisinin RX karakteristiğine tek bir yazma ya da TX karakteristiğinde tek bir bildirimdir; platform kodu bunları seri porttaki gibi çerçeveler. Cihaz önce eşleştirilmiş olmalıdır. Düğüm bir seferde tek komut gönderir ve yanıtı için en çok 5 saniye bekler; bu arada cihazın gönderdiklerini de alır. Bir komut yalnızca kendi türlerindeki yanıtları ya da bir hatayı kabul eder: diğer yanıtlar, cihazın ne olduğunu anlayan sorunun ikinci cevabı gibi, daha önce sorulmuş bir şeye geç gelen cevaplardır ve atlanır. Kanal yuvası sayısını (`CMD_DEVICE_QUERY`), cihazın adını ve radyo ayarlarını (`CMD_APP_START`) ister, cihazın saatini ayarlar (`CMD_SET_DEVICE_TIME`) ve CarryChat'in kanalını bulur.
- **MeshCore kanalı.** Çerçeveler `#carrychat` hashtag kanalında gider; her MeshCore hashtag kanalında olduğu gibi kanalın gizli anahtarı, adının SHA-256 özetinin ilk 16 baytıdır. Bu kanala sahip olmayan cihaz onu ilk boş yuvasına (adı olmayan ve gizli anahtarı tamamen sıfır olan) alır. Tüm yuvaları dolu olan cihaz kanallarını korur ve hiçbir şey göndermez; uygulamalar bunu belirtir. Kanalın anahtarını herkes türetebilir; CarryChat zarfları zaten uçtan uca şifrelidir.
- **MeshCore paketleri.** Her çerçeve, her tekrarlayıcı üzerinden her yöne yayılan (flood) tek bir kanal datagramıdır (`CMD_SEND_CHANNEL_DATA`); veri türü, bir tür kaydedilene kadar MeshCore'un test aralığından `0xFFCC`'dir. Bir datagram en çok 165 bayt taşır (`MAX_GROUP_DATA_LENGTH`); bu yüzden bundan uzun zarflar parçalar hâlinde gider (§7). Bir cihaz yanıt verene kadar düğüm zarfları bu boyuta göre böler; bu boyut iki cihaz türüne de sığar. Duyulan datagramlar cihazda bekler: düğüm, cihaz bekleyen datagram olduğunu söylediğinde ve her 60 saniyede bir bunları çeker (`CMD_SYNC_NEXT_MESSAGE`) ve CarryChat'in kanalında CarryChat'in veri türüyle gelenleri alır. Diğer mesajlar MeshCore'un kendi uygulamaları içindir ve CarryChat bağlantıyı tuttuğu sürece atılır. Cihazın dolu gönderme kuyruğunun reddettiği çerçeve 2 saniye sonra yeniden ilk sırada gider.
- **MeshCore cihazının duydukları.** Düğüm, bağlandığında ve her 10 dakikada bir cihazın bildiği cihazları (`CMD_GET_CONTACTS`), arada da cihazın ilettiği advert mesajlarını okur: adlarını, cihazın onları en son ne zaman duyduğunu ve yol biliniyorsa kaç atlama uzakta olduklarını. Tekrarlayıcılar varsayılan olarak her 47 saatte bir advert yayınlar; bu yüzden uygulamalar son iki günde duyulan cihazları gösterir.
- **Bölünmüş ufuk.** İki ağ da paketleri kendileri yeniden yayınlar (Meshtastic en çok 7 atlama sınırıyla); bu yüzden düğüm, LoRa'da duyduğu bir zarfı ya da `tombstone`'u, bütün ya da parçalar hâlinde, yeniden LoRa üzerinden göndermez (§8). Bunları diğer bağlantılarına iletir. Bir çerçeve yüzünden düğümün gönderdikleri ise (örneğin kendisine gelen bir mesajın alındısı) LoRa üzerinden gider.
- **Yayın süresi.** `EU_868` bölgesinde Meshtastic 869.4–869.65 MHz aralığını kullanır; burada yasal görev döngüsü (duty cycle) sınırı %10'dur ve erişim alanındaki her düğüm bu yayın süresini paylaşır. Düğüm kendi yayın süresini zamanın %2.5'inin altında, en çok 20 saniyelik patlamalarla tutar; her paketin havada kalma süresini cihazın modem ayarlarından ve ağın başlığı için ayrılan paydan hesaplar: Meshtastic için 22 bayt, MeshCore için 24 bayt (başlık, yol uzunluğu, kanal özeti, MAC, veri türü ve uzunluğu, dolgu). Cihaz hangisi olursa olsun aynı bütçe geçerlidir. LongFast'ta dolu bir paket yaklaşık 2.2 saniye sürer, yani bir patlamaya yaklaşık dokuz paket sığar. Önce düğümün kendi zarfları ve `tombstone`'ları gider. Başkaları için taşınan en çok 32 çerçeve yayın süresi bekler; bunun ötesinde en eskisi atılır.

### 9.3 Nostr ile internet

`crates/carrychat-node/src/internet.rs` içinde gerçeklenmiştir.

- **Olaylar.** Her zarf ve `tombstone`, `9378` türünde tek bir Nostr olayı (event, NIP-01) olarak gider; bu, Nostr'a kayıtlı olmayan geçici bir numaradır. İçerik, dolgusuz base64url ile kodlanmış çerçevedir (§7). Zarf olayının etiketi `["x", hex(tag)]`, `tombstone` olayınınki `["b", hex(bundle_id)]` olur.
- **Etiketler.** Çift zarfı kendi alıcı etiketiyle yayımlanır. Tanıtım, aynı zamanda kendi etiketi de olan (§5.5) alıcının posta kutusu etiketiyle yayımlanır; bu etiketi alıcının kartına sahip olan herkes hesaplayabilir. Rastgele etiketli eski bir tanıtımı yapmış olan düğüm de onu posta kutusu etiketiyle yayımlar:

  ```
  mailbox_tag(pk, d) = HMAC(H("carrychat/v0/mailbox" || pk), u32(d))[0..8]
  ```

  `d`, tanıtımın oluşturulduğu gündür.
- **Anahtarlar.** Her olay tek kullanımlık bir anahtarla (`HMAC(node_secret, content)`) imzalanır (BIP-340); `node_secret` düğümün sakladığı rastgele bir gizli değerdir. Anahtarlar kullanıcıya ya da birbirine bağlanamaz; aynı düğüm aynı çerçeveyi yeniden yayımladığında aynı olay oluşur ve aktarıcılar bunu kopya olarak atar.
- **Zaman.** Bir zarf olayının `created_at` değeri `expires_at − 604800`'dür; bu asla zarfın oluşturulma zamanından sonra değildir. `tombstone` olayınınki yayımlandığı zamandır.
- **Abonelik.** Düğüm her aktarıcıdan, etiket penceresi (§4.1) boyunca tüm kişilerinin çift etiketleri ve kendi posta kutusu etiketleriyle `{"kinds": [9378], "#x": [...], "since": now − 691200}` ister; ayrıca hâlâ alındı bekleyen kendi zarflarının kimlikleriyle `{"kinds": [9378], "#b": [...]}` ister. Aboneliği kişileri değiştiğinde, gönderim yaptığında ve her 10 dakikada bir yeniler.
- **Bağlantılar.** Düğüm her 20 saniyede bir her aktarıcıya ping gönderir, 40 saniye sessiz kalan bağlantıyı bırakır ve yeniden bağlanır. Gönderimden hemen sonra da her aktarıcıya ping gönderir ve 5 saniye içinde yanıt vermeyen bağlantıyı bırakır. Yeniden bağlandığında bekleyen zarflarını bir kez daha yayımlar. Aksi hâlde başka bir ağa geçen bir bilgisayar, ölü bir bağlantıyı fark etmeden dakikalarca tutabilir ve ona gönderdiklerini kaybedebilir.
- **Düğümün yayımladıkları.** Kendi zarfları (gönderdiğinde ve her bağlantıda yeniden), aldığı zarfların `tombstone`'ları ve ağ geçidi olarak, taşıdığı ve başka bir bağlantıdan gelmiş her zarf: bir aktarıcı bağlıysa geldiği anda ve bir aktarıcı onu `OK` ile kabul edene kadar her bağlantıda. Bir aktarıcının reddettiği zarf bir sonraki bağlantıda yeniden gönderilir.
- **Düğümün aldıkları.** Yalnızca kendisine gelen zarflar ve `tombstone`'lar. Bir aktarıcıdan gelen ve açılamayan zarf taşınmaz, atılır; böylece bir aktarıcı düğümün deposunu dolduramaz.
- **Tor.** Düğüm aktarıcılara Tor üzerinden ulaşabilir; her aktarıcı kendi devresinde olur ve aktarıcının ana makine adı çıkış düğümünde çözülür. Böyle bir düğüm asla doğrudan bağlanmaz: Tor çalışmadığı sürece interneti hiç kullanmaz.
- **Aktarıcılar** etiketleri, boyutları ve zamanlamayı, düğüm Tor üzerinden gelmiyorsa IP adresini de görür. Ayrıca tek bir bağlantının hangi etiketleri birlikte istediğini görürler; bu da bir gün içinde bir düğümün kişilerine ait etiketleri birbirine bağlar.

Varsayılan aktarıcılar şunlardır: `wss://relay.damus.io`, `wss://nos.lol` ve `wss://relay.primal.net`. İnternet erişimi olmayan düğümler için veri çekme (bir ağ geçidi üzerinden aşağı yönlü iletim, downlink) hâlâ açık bir sorudur (§11).

### 9.4 Dosya

`crates/carrychat-node/src/file.rs` içinde gerçeklenmiştir; dosya uzantısı `.carrychat`. Bir dosya `"CRCH" || 0x00 || frames` biçimindedir; her çerçevenin önüne `u16` uzunluğu eklenir. Dışa aktarma deponun zarflarını ve `tombstone`'larını yazar. İçe aktarma her çerçeveyi, tam olarak başka bir bağlantıdan gelmiş gibi depoya verir.

### 9.5 Bluetooth LE

Masaüstü için btleplug aracılığıyla `crates/carrychat-node/src/bluetooth.rs` içinde, Android'de ise her bağlantının baytlarını düğüme veren Android uygulamasında (`BleLink.kt`) gerçeklenmiştir.

- **Yayın (advertisement).** Düğüm `43524348-a0c5-4a3e-9b31-6f2d8e7c1a01` servisini yayınlar. Tarama yanıtı, o uuid için servis verisi olarak düğümün 8 baytlık oturum kimliğini (yerel ağ işaret paketlerindekiyle aynı, §9.1) taşır. Ad yayınlamaz.
- **Akış.** Servisin tek bir karakteristiği vardır, `43524348-a0c5-4a3e-9b31-6f2d8e7c1a02`; write, write without response ve indicate özelliklerine sahiptir. Bağlanan düğüm (GATT istemcisi) ona yanıtlı yazar, bağlanılan düğüm (GATT sunucusu) da onun üzerinden indication gönderir; böylece her yönde bir bayt akışı oluşur. Her yazma ya da indication akıştan en çok MTU − 3 bayt (en fazla 512) taşır ve bir sonraki, karşı taraf bir öncekini onaylayana kadar bekler. Aynı anda daha fazlası yoldayken bildirimler bir telefonun Bluetooth kuyruğunu, karşı tarafın yazmalarına yanıt veremeyecek kadar doldurdu ve bir yanıt 30 saniye geciktiğinde (ATT zaman aşımı) bağlantı koptu. MTU'dan uzun bir yazma, hazırlanmış yazmalar (prepared writes) olarak gelir ve yürütüldüğünde akışa katılır. Indication'lara abone olmak akışı açar; bağlantıyı kesmek akışı sona erdirir.
- **Eşitleme.** Akış üzerinde §9.1'deki eşitleme çalışır: önlerine `u16` uzunlukları eklenmiş çerçeveler, HELLO, HAVE, WANT, ardından ortaya çıktıkça gönderilenler. Yerel ağ üzerinden zaten bağlı olan bir düğüme Bluetooth üzerinden yeniden bağlanılmaz; yine de kurulan bir bağlantı HELLO'dan sonra kapanır.
- **Kim bağlanır.** Yerel ağda olduğu gibi oturum kimliği küçük olan düğüm bağlanır; diğeri, bağlantı kurulmadan karşıyı üç kez gördükten sonra bağlanır. Yayın yapamayan düğüm her durumda bağlanır. Masaüstü bilgisayarlar btleplug ile yayın yapamaz; bu yüzden telefonlara bağlanırlar ve diğer masaüstlerine yerel ağ üzerinden ulaşırlar.
- **Masaüstü bilgisayarlar** her seferinde 512 bayt olmak üzere yanıtlı yazar: macOS kuyruğu dolunca yanıtsız yazmaları atar ve bu, akışı bozardı. Bir tarama her cihazı bir kez bildirdiği için her 5 saniyede bir o ana kadar görülen cihazlara bakar ve aralarındaki bağlı olmayan CarryChat düğümlerine bağlanırlar.
- **Android** bağlantıyı, düğümü canlı tutan ön plan servisinde çalıştırır; böylece uygulama kapalıyken de çalışır. Bluetooth izinlerini `neverForLocation` ile ister.

### 9.6 Wi-Fi Aware

Android uygulamasında (`WifiAwareLink.kt`) gerçeklenmiştir; uygulama, Bluetooth'takine benzer şekilde her bağlantının baytlarını düğüme verir. Wi-Fi Aware (Neighbor Awareness Networking) destekleyen telefonlar, erişim noktası ve internet olmadan doğrudan Wi-Fi üzerinden, Bluetooth'tan daha uzağa ve çok daha hızlı bağlanır. Masaüstü bilgisayarlar katılmaz.

- **Keşif.** Her telefon `carrychat` adlı servisi yayımlar ve ona abone olur. Yayımın servise özgü bilgisi, düğümün 8 baytlık oturum kimliğidir.
- **Kim bağlanır.** Diğer bağlantılarda olduğu gibi düğüm oturum kimliğine göre seçer (§9.1). Bağlanan abone önce yayımcıya kendi oturum kimliğini içeren bir keşif mesajı gönderir. Yayımcı bunun üzerine bir TCP portunu dinler ve portu bildirerek o aboneye bir veri yolu ister; abone de yayımcıya bir veri yolu ister, portu ve yayımcının IPv6 adresini bu yoldan öğrenir ve bağlanır. İki veri yolu da `carrychat-wifi-aware` geçiş parolasını kullanır: üzerindeki her şey zaten uçtan uca mühürlü olsa da bağlantı şifrelidir.
- **Eşitleme.** TCP bağlantısı §9.1'deki eşitlemeyi taşır.
- **Android** bağlantıyı Android 10'dan itibaren ön plan servisinde çalıştırır. Android 13'ten itibaren `neverForLocation` ile `NEARBY_WIFI_DEVICES` iznini, daha önceki sürümlerde ise Wi-Fi Aware keşfinin orada gerektirdiği konum iznini ister. Veri yolu istemenin gerektirdiği `CHANGE_NETWORK_STATE` iznine sahiptir; Android'in reddettiği bir veri yolu, hiç gelmemiş bir veri yolu gibi sona erer.

### 9.7 Yakınımdakiler

Bir düğüm yakındaki insanlara görünür olabilir. Görünür bir düğüm, yerel ağ, Bluetooth, Wi-Fi Aware ya da Wi-Fi Direct üzerinden doğrudan bağlı olduğu düğümlere kim olduğunu söyler, ama yalnızca kendilerinin de görünür olduğunu söylemiş olanlara: iki kişi birbirini ancak ikisi de bu özelliği açtığında görür ve açmamış bir düğüm hiç kimsenin kartını asla görmez. Bununla ilgili hiçbir şey aktarılmaz.

| Tür | Anlam |
| --- | --- |
| `0` ASK | "Görünürüm: sen kimsin?" |
| `1` ME | "Bu benim", ardından gönderenin kişi kartı (§3) |
| `2` GONE | "Artık görünür değilim." |

Görünür bir düğüm her yeni bağlantıda ve görünür hâle geldiğinde bağlı her düğüme ASK gönderir. Görünür bir düğüm ASK'e bağlantı başına bir kez ME ile yanıt verir ve henüz yanıtlamadığı bir ME'ye kendi ME'siyle yanıt verir. Görünür olmayan düğüm ASK ve ME'yi yok sayar. Görünür olmayı bırakan düğüm GONE gönderir ve herkesi unutur; diğerleri onu GONE geldiğinde ya da bağlantı sona erdiğinde unutur. Yakında görülen birini kişi olarak eklemek, her kartta olduğu gibi onun kartını kullanır (§5.5).

### 9.8 Wi-Fi Direct

Android uygulamasında (`WifiDirectLink.kt`) gerçeklenmiştir; uygulama, Wi-Fi Aware'dekine benzer şekilde her bağlantının baytlarını düğüme verir. Wi-Fi Direct, Wi-Fi Aware desteklemeyen telefonlara da ulaşır: neredeyse her Android telefonda vardır. Masaüstü bilgisayarlar katılmaz.

- **Keşif.** Her telefon bir Wi-Fi Direct DNS-SD servisi duyurur: örnek adı `CarryChat`, türü `_carrychat._tcp`; TXT kaydında `s` (düğümün onaltılık oturum kimliği) ve `h` (telefon bir grup kurmuşken `1`, aksi hâlde `0`) bulunur. Her telefon bu servisi her 30 saniyede bir arar.
- **Grubu kim kurar.** Bir Wi-Fi Direct grubuna katılmak normalde grubun sahibi olan telefonun kullanıcısına sorulur. Burada grubu kuran telefon özerk (autonomous) bir grup oluşturur; diğer telefon bu grubun ağ adını (`DIRECT-CC-` ve ardından onaltılık oturum kimliği) ve geçiş parolasını (`carrychat-` ve ardından aynısı) duyurudan türetir ve gruba bunlarla katılır; böylece kimseye bir şey sorulmaz. İki telefondan oturum kimliği büyük olan grubu kurar, çünkü düğüm önce diğerinin bağlanmasına izin verir (§9.1). Bir telefon, bir gruba katılmamışsa daha küçük bir oturum kimliği duyduğunda grup kurar; kendisi grup kurmamışsa, düğüm bağlanmayı seçtiğinde daha büyük oturum kimliğine sahip bir kurucuya katılır. 2 dakika içinde kimsenin katılmadığı gruptan vazgeçilir ve telefon 10 dakika boyunca yeniden grup kurmaz. Bir gruptaki telefon Wi-Fi Direct üzerinden başka hiçbir telefonla konuşmaz.
- **Eşitleme.** Grubu kuran telefon 43524 numaralı TCP portunu dinler; katılan telefon grup sahibinin adresine bağlanır ve bunun üzerinden §9.1'deki eşitlemeyi çalıştırır. Eşitleme bitince gruptan ayrılır. Üzerindeki her şey zaten uçtan uca mühürlü olsa da geçiş parolası bağlantıyı şifreler.
- **Android** bağlantıyı, ağ adı ve geçiş parolasıyla katılmanın gerektirdiği Android 10'dan itibaren ön plan servisinde çalıştırır. Wi-Fi Aware ile aynı izinleri ister.

### 9.9 Reticulum

`crates/carrychat-node/src/reticulum.rs` içinde gerçeklenmiştir; Reticulum'un iletim biçimi (wire format), referans gerçeklemesi RNS 1.5.5 izlenerek `reticulum/wire.rs` içindedir. Düğüm bir [Reticulum](https://reticulum.network) ağındaki diğer CarryChat düğümlerine, o ağ hangi ortam üzerinde çalışıyorsa onun üzerinden, bağlandığı tek bir Reticulum düğümü aracılığıyla ulaşır.

- **Bağlantı.** Düğüm TCP üzerinden bir Reticulum TCP sunucu arayüzüne (örneğin aktarımı etkin bir `rnsd`) ya da aynı bilgisayardaki paylaşılan `rnsd` örneğine (macOS ve Windows'ta `127.0.0.1:37428`) bağlanır. Her paket, Reticulum'un TCP arayüzlerinin çerçevelediği gibi çerçevelenir: `0x7E`, `0x7D`'nin `7D 5D` ve `0x7E`'nin `7D 5E` olarak kaçışlandığı paket, `0x7E`. Düğüm sıradan bir örnektir: başkaları için hiçbir şey yönlendirmez. Bağlantı sona erdiğinde 10 saniye sonra yeniden bağlanır.
- **Kimlik ve hedef.** Düğümün kendine ait bir Reticulum kimliği vardır: ilk seferde oluşturulup saklanan, CarryChat kimliğiyle hiçbir ilgisi olmayan bir X25519 ve bir Ed25519 anahtarı. Hedefi, bu kimliğin `carrychat.frames` tekil hedefidir (single destination): `SHA-256(name_hash || identity_hash)[0..16]`; burada `name_hash = SHA-256("carrychat.frames")[0..10]` ve `identity_hash = SHA-256(X25519 public || Ed25519 public)[0..16]`.
- **Duyurular.** Düğüm hedefini uygulama verisi olmadan her bağlantıda, sonrasında saatte bir ve bilmediği bir CarryChat hedefi duyduğunda bir kez daha duyurur, çünkü düğümler bir istemci bağlanmadan önce yapılmış duyuruları iletmez. Bu karşılama duyuruları 5'lik bir paydan çıkar; pay her 10 dakikada bir artar, bu da aktarım düğümlerinin bir hedefin duyurularını geri tutmadan önce ilettiği kadardır. Beklemesi gereken bir karşılama atlanmaz, geç yapılır ve o arada duyulan bütün yeni gelenleri kapsar. Her duyuru bir öncekinden daha geç bir zaman damgası taşır. Hedefi için gelen bir yol isteğini, yol yanıtı olarak işaretlenmiş bir duyuruyla yanıtlar. Aktarım düğümleri, saatte birden sık duyuru yapan bir hedefin duyurularını birkaç taneden sonra iletmeyi bırakır.
- **Kimler var.** Ad özeti `carrychat.frames`'inkiyle aynı olan, imzası geçerli ve özeti anahtarla eşleşen diğer hedeflerin duyuruları CarryChat düğümleridir; bir hedef ilk duyurulduğu anahtarı korur. Düğüm bunlardan en çok 64 tanesini yeniden başlatmalar arasında saklar ve her bağlantıda yollarını ister (yeni bir rastgele etiketle `rnstransport.path.request`'e bir yol isteği, 0.5 saniyede bir), çünkü bağlandığı düğüm onların duyurularını iletmemiş olabilir. Son altı saat içinde duyurulmuş ya da bir yol isteğiyle bulunmuş olanlara gönderim yapar. Son zamanlarda duyulmamış birine önce, hâlâ alındı bekleyen kendi zarflarını ve kendisine teslim edilen son 32 zarfın `tombstone`'larını gönderir; bağlantı başlamadan önce yazılan mesajlar gibi bunları kaçırmış olabilir.
- **Çerçeveler.** Reticulum üzerinden yalnızca zarflar ve `tombstone`'lar gider, asla dosya dilimleri gitmez. Her çerçeve, her CarryChat hedefine tek bir veri paketi olarak, Reticulum'un tekil bir hedefe şifrelediği gibi o hedefin kimliğine şifrelenerek gider: yeni bir X25519 anahtar çifti; paylaşılan gizli değer, alıcının kimlik özeti tuz (salt) olarak kullanılıp info olmadan HKDF-SHA256'dan geçirilir ve 64 bayt elde edilir; ilk 32 bayt HMAC-SHA256 anahtarı, son 32 bayt PKCS#7 dolgulu AES-256-CBC anahtarıdır; veri `ephemeral public key || IV || ciphertext || HMAC(IV || ciphertext)`. Bir paket en çok 383 bayt çerçeve taşır; bu yüzden daha uzun zarflar parçalar hâlinde gider (§7). Bir aktarım düğümü üzerinden duyurulmuş hedef, paketi o düğümü bir sonraki atlama olarak belirten ikinci başlıkla alır; aktarım düğümü, daha uzaktaki bir hedef için gelen ve bunu yapmayan paketi atar. Düğüm hiçbir kanıt (proof) istemez ve özetini daha önce gördüğü paketleri atar.
- **Bölünmüş ufuk.** Reticulum üzerinden duyulan bir çerçeve yeniden Reticulum üzerinden gönderilmez: göndereni onu bildiği her CarryChat düğümüne zaten göndermiştir. Diğer bağlantılara iletilir. Bir çerçeve yüzünden düğümün gönderdikleri ise (örneğin bir alındı) Reticulum üzerinden gider.

## 10. Güvenlik

**v0 şunları sağlar:**

- mesaj içeriğinin, onu hangi yol taşırsa taşısın, uçtan uca gizliliği ve bütünlüğü,
- aktarıcılara karşı ilişkilendirilemezlik: bir çift anahtarı olmadan göndereni ya da alıcıyı anlayamaz, bir çiftin trafiğini günler ya da yönler arasında ilişkilendiremezler,
- sahtelenemeyen teslim alındıları,
- kapalı bir gelen kutusu: sana yalnızca kartına sahip olan biri yazabilir ve ilk mesajı, kabul ettiğin ya da reddettiğin bir istektir,
- yalnızca güncel üyelerin okuyabildiği ve hiçbir üyenin başka bir üye adına sahteleyemediği grup mesajları (§5.7),
- gönderileri yazarlarının kartlarına ya da kanallar arasında birbirine bağlanamayan ve hiç kimsenin başka bir yazar adına sahteleyemediği herkese açık kanallar (§5.8).

**v0 şunları sağlamaz:**

- **Gizli tanıtımlar.** Bir tanıtım göndericisine bağlanamaz, ama alıcının kartına sahip olan herkes, her bağlantıda, etiketinden (alıcının posta kutusu etiketi) onun alıcıya gittiğini anlayabilir. Boyutu da onu çoğu çift zarfından ayırır.
- **Her zarf için ileri gizlilik.** Ratchet mesajlarında (§5.6) vardır: sonradan bir cihazdan alınan anahtar, cihazın daha önce almış olduğu kaydedilmiş mesajları açmaz; her yönde birer yanıt gittikten sonra sonrakileri de açmaz. Tanıtımlarda, oturumsuz `K_enc` zarflarında ve sürüm 1'den önceki zarflarda yoktur; ayrıca bir cihazın X25519 gizli anahtarını ele geçiren, o cihaza karşı kişilerinden herhangi birinin kimliğine bürünebilir. İleri gizlilik kaydedilmiş trafiği korur, cihazda tutulan geçmişi değil.
- **Radyo üst verisi gizliliği.** Bir LoRa yayını, yayını yapan Meshtastic ya da MeshCore düğümünü ve yön bulma ekipmanına onun konumunu açığa çıkarır. v0 dolgu yapmadığı için zarf boyutları ve zamanlama görünürdür.
- **Ağda yeniden oynatmaya karşı koruma.** Kaydedilmiş bir zarf, süresi dolana kadar yeniden ağa sokulabilir. Alıcılar kopyaları atar, ama yeniden oynatmalar yine de yayın süresi harcar.
- **Aktarıcılarda parça bütünlüğü.** Aktarıcılar parçaları doğrulayamaz; bu yüzden sahte bir parça, zaman aşımına kadar o zarfın yeniden birleştirilmesini bozar.
- **Herkese açık kanallarda gizlilik.** Bir kanalın adını ve varsa parolasını bilen herkes, daha önce kaydettiği gönderiler dahil kanalın gönderilerini okur. Anahtarlar doğrudan paroladan türetildiği için parola ancak tahmin edilmesi zor olduğu ölçüde güçlüdür. Kanal gönderilerinde ileri gizlilik ve teslim alındısı yoktur.
- **Gruptan anında çıkarma.** Bir çıkarmayı henüz görmemiş üyeler, çıkarılan üyenin elindeki zincirlerle yazmayı sürdürür. Grup zarflarında ayrıca teslim alındısı yoktur ve bir grubun yöneticisi ile üye listesi her üyeye görünür.
- **Güvenilir zaman.** İnternet erişimi olmayan cihazların saatleri yanlış olabilir. Etiket penceresi bir günlük kaymayı karşılar; süre dolumu denetimleri yerel saati kullanır.

## 11. Açık sorular

1. İnternet erişimi olmayan düğümler için, ağ geçidine kim olduklarını söylemeden, bir ağ geçidi üzerinden aşağı yönlü iletim (downlink).
2. LoRa çerçeveleri için dolgu: daha az boyut sızıntısına karşılık daha fazla yayın süresi.
3. Uzaktan kişi alışverişi.
4. HF radyo. Amatör bantlar bir mesajın anlamını gizleyen şifrelemeyi yasaklar; bu yüzden HF için, kullanıcının açıkça herkese açık olarak işaretlediği mesajlarla sınırlı, ayrı, imzalı ama şifresiz bir mesaj türü gerekir.

## 12. Sabitler

| Ad | Değer |
| --- | --- |
| Protokol sürümü | `0` |
| En uzun ad | 32 bayt |
| En uzun zarf | 4096 bayt |
| Zarf ek yükü | 67 bayt, ratchet başlığıyla 104 |
| Tanıtım ek yükü | 164 bayt artı kart |
| Grup zarfı ek yükü | 135 bayt |
| Bir gruptaki üyeler | Yönetici dahil en çok 16 |
| Kanal zarfı ek yükü | 164 bayt artı takma ad |
| Kanal zarfı ömrü | En çok 86400 s (1 gün), atlama sınırı 4 |
| En uzun kanal adı ve takma ad | 32 ve 20 bayt |
| En uzun dosya | 1048576 bayt (1 MiB), 4000 baytlık dilimler hâlinde |
| Varsayılan ömür | 259200 s (3 gün) |
| En uzun ömür | 604800 s (7 gün) |
| Varsayılan atlama sınırı | 8 |
| Etiket penceresi | `today − 7` ile `today + 1` arası |
| Meshtastic MTU | 233 bayt |
| MeshCore MTU | 165 bayt |
| MeshCore kanalı, veri türü | `#carrychat`, `0xFFCC` |
| LoRa yayın süresi bütçesi | Zamanın %2.5'i, en çok 20 s'lik patlamalar |
| Zarf başına en çok parça | 32 |
| Yeniden birleştirme zaman aşımı | 21600 s (6 sa) |
| Bluetooth LE servisi | `43524348-a0c5-4a3e-9b31-6f2d8e7c1a01` |
| Bluetooth LE akış karakteristiği | `43524348-a0c5-4a3e-9b31-6f2d8e7c1a02` |
| Wi-Fi Aware servis adı | `carrychat` |
| Wi-Fi Aware veri yolu geçiş parolası | `carrychat-wifi-aware` |
