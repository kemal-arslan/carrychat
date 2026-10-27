---
# gstack: design-md-format=spec
name: CarryChat
description: Açık renkli, sakin bir mesajlaşma uygulaması. Beyaz yüzeyler, tek bir mavi vurgu rengi, tek bir yazı tipi; yalnızca CarryChat'in gösterdiği birkaç şey de (bir mesajın nasıl yol aldığı, hangi bağlantıların açık olduğu) aynı sade dille çizilir.
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

[English](DESIGN.md) · **Türkçe**

İki sürüm birbirinden farklıysa İngilizce sürüm esas alınır.

## Genel Bakış

**Yaratıcı ana fikir:** araya girmeyen, temiz bir telefon uygulaması. İnsanlar CarryChat'i birkaç kelime okumak ve göndermek için açar, çoğu zaman pili azken ve internet yokken; arayüz birkaç saniyede tanıdık gelmeli ve hiçbir zaman endişe verici görünmemeli.
**Ürün bağlamı:** Android, macOS, Windows ve Linux için çevrimdışı öncelikli, uçtan uca şifreli bir mesajlaşma uygulaması. Mesajlar, alıcıya bir yol bulunana kadar saklanır ve yerel ağ, Bluetooth, LoRa, internet ya da bir dosya üzerinden cihazdan cihaza taşınır. Saniyeler içinde de ulaşabilirler, günler içinde de.
**Yüzeye göre mod:** her yerde Operate (kullanım odaklı): sohbet listesi, konuşma, yazma alanı, kişiler, Ağ sekmesi, ayarlar ve pencereler (alt sayfalar).
**Referans siteler:** dribbble.com üzerindeki güncel mobil mesajlaşma çalışmaları (açık renkli sohbet listeleri, yüzen eylem düğmeleri, filtre düğmeleri, alttan açılan pencereler) ve Android'deki Material 3 gezinmesi.
**Temel özellikler:**
- Beyaz ve açık gri, tek vurgu rengi; renk geçişi, parlama ya da cam efekti yok.
- Her şey için tek yazı tipi.
- Bekleyen bir mesaj beklediği gibi görünür, asla başarısız olmuş gibi değil.
- Bağlantılar (yerel ağ, Bluetooth, internet) bir durum çubuğunda değil, kendi sekmelerinde yer alır.
- İletişim kutuları, aşağı sürüklenerek kapatılabilen alttan açılan pencerelerdir.

## Renkler

**Strateji:** Ölçülü. Arayüzü nötr renkler taşır; vurgu rengi, üzerinde işlem yapılabilen şeyleri ve kullanıcıya ait olanı işaretler: birincil düğmeler, giden balon, okunmamış sayıları, etkin sekme ve yüzen düğme. Başarı yeşili yalnızca çalışan bir bağlantıda ("2 cihaz yakında") ve doğrulanmış bir kişide görünür. Hata kırmızısı gerçek hatalara ayrılmıştır (çalışmayan bir bağlantı, fazla uzun bir mesaj); bekleyen bir mesajda asla kullanılmaz.
**Açık ya da koyu:** kullanıcı Ben sekmesinde Açık ya da Koyu seçmedikçe uygulama sistemi izler; Android'de durum ve gezinme çubukları da aynı yüzey rengini alır. Ana tasarım açık temadır: dışarıda iyi okunur ve insanların bir mesajlaşma uygulamasından beklediğine uyar. Gece teması düz koyu griler kullanır (renk tonu yok, renk geçişi yok) ve aynı hiyerarşiyi korur: önce surface, balonlar ve alanlar için surface-muted, ayırıcılar için line. Vurgu dolgusu aynı kalır, böylece üzerindeki beyaz metin kontrastını korur; koyu yüzeyde metin olarak kullanılan vurgu rengi, daha açık olan accent-text-night rengine geçer.
Vurgu rengi üzerindeki beyaz 5.2:1, surface üzerindeki text-muted 5.0:1 kontrast verir; text-faint yalnızca zaman damgaları ve yer tutucu metinler içindir.

## Tipografi

- **Figtree**, değişken (variable) yazı tipi, her yerde: başlıklar 750, adlar 650, gövde metni 400 kalınlıkta. Küçük boyutlarda da sıcak kalan sade, geometrik bir sans-serif yazı tipidir ve Türkçeyi tam olarak kapsar (ç ğ ı İ ö ş ü).
- Başlıklar büyük (28px) ve sıkıdır; ikinci bir başlık yazı tipi yoktur.
- Mesaj metni telefonlarda 16px, masaüstünde 15px boyutundadır.
- Saatler ve sayılar tablo rakamları (tabular figures) kullanır. Parmak izleri sistemin eş aralıklı (monospace) yazı tipiyle gösterilir.
- Her yerde cümle düzeni (yalnızca ilk harf büyük); tamamı büyük harfli etiket yok.
- Figtree, SIL Open Font License altındadır ve `@fontsource-variable/figtree` ile uygulamanın içinde gelir; böylece internet yokken de, uygulamanın `font-src 'self'` politikası altında da aynı görünür.

## Yerleşim

- **Telefon (640px ve altı):** bir seferde tek ekran.
  - Dört hedefli bir alt gezinme çubuğu (Material 3): Sohbetler, Kişiler, Ağ, Ben. Etkin olanın dolu simgesinin arkasında renklendirilmiş bir hap şekli bulunur.
  - Bir konuşmayı açmak sekmenin üstünü kaplar ve çubuğu gizler; Android'in geri tuşu ve geri oku geri döndürür.
  - Android'in geri tuşu en üstte olanı kapatır: önce pencereyi, sonra konuşmayı, sonra Sohbetler dışındaki sekmeyi. Sohbetler'de, Ana ekran tuşu gibi, uygulamayı arka plana gönderir.
  - Sohbetler'de (yeni bir şey başlatmak için) ve Kişiler'de (kişi eklemek için) çubuğun üstünde yüzen bir düğme durur.
- **Masaüstü:** aynı dört hedef soldaki 360px genişliğindeki bir sütunda yer alır, gezinme çubuğu sütunun altındadır; açık konuşma kalan alanı doldurur.
- **Sohbetler:** Ara ve "Kartım" düğmeleriyle büyük başlık, Ağ'ı açan tek satırlık bağlantı özeti, filtre düğmeleri (Tümü, Okunmamış, Gruplar, Kanallar), ardından ayırıcısız satırlar: 50px avatar, ad, saat, son mesaj ve okunmamış sayısı.
- **Konuşma:** geri düğmesi, avatar, ad ve tek satırlık bağlam içeren bir üst bilgi, ardından balonlar, ardından tek satırlık bir yazma alanı.
- **Ağ:** yakında kaç cihaz olduğunun özeti, ardından her bağlantı için durumu ve anahtarıyla gruplanmış satırlar, ardından elden ele taşıma seçenekleri (Dosyaya aktar, Dosya al, Adrese bağlan).
- **Ben:** kimlik (ad, parmak izi, kart), ardından gruplanmış satırlarda ayarlar.
- Kenar boşlukları telefonlarda 20px; satırlar en az 44px yüksekliğinde; boşluklar 4px adımlarla artar.

## Yükselti ve Derinlik

Düz. Yüzeyler gölgeyle değil, dolguyla (surface ve surface-muted) ve ince çizgilerle (line) ayrılır. İki istisna vardır, ikisi de yumuşak, kaydırılmış bir gölgeyle: listenin üzerinde yüzen eylem düğmesi ve karartılmış bir örtünün (scrim) üzerine yükselen pencereler.

## Şekiller

- Balonlar 20px yarıçapla yuvarlatılır; art arda gelen balonların sonuncusunda gönderene en yakın köşe 6px olur.
- Gruplanmış satırlar, kartlar ve ağ özeti 20px; metin alanları 12px; yüzen düğme 18px; pencereler üst kenarda 28px kullanır.
- Düğmeler, çipler, yazma alanı ve rozetler tam hap şeklindedir. Avatarlar dairedir: addan seçilen bir rengin açık tonu üzerinde, aynı renkte baş harf. Gruplarda kişiler simgesi, kanallarda "#" görünür.

## Bileşenler

- **Düğmeler:** birincil (vurgu rengi dolgu), ikincil (surface-muted dolgu), tehlikeli (surface-muted üzerinde hata rengi metin; düz hata rengi dolgu yalnızca son onayda). Klavye odağında (focus-visible) 2px dışarıda 2px kalınlığında bir vurgu rengi halka çizilir; basılıyken 6% koyulaşır; devre dışıyken opaklık 40% olur.
- **Mesaj durumları** (giden):
  - Yol bekleniyor: balon beyazdır ve kesik çizgili, vurgu renginde bir çerçevesi vardır; alt bilgisinde kum saatiyle birlikte "Yol bekleniyor" yazar. Sakin bir durumdur, hata değildir. Mesaj bir bağlantıdan yola çıktığında ama henüz ulaşmadığında alt bilgide "Yolda" yazar.
  - Teslim edildi: tek tik. Okundu: çift tik.
  - Alt bilgi, mesajın yola çıktığı her bağlantı için ilk kullanım sırasına göre küçük bir simge gösterir.
- **Mesajın yolculuğu:** kendi balonlarımızdan birine dokunmak bir pencere açar: mesajın ne kadar süre yolda kaldığı, yakındaki kaç cihazın onu aldığı, internetin kullanılıp kullanılmadığı ve bir zaman çizelgesi: yazıldı, yola çıktığı her bağlantı (cihaz ya da sunucu sayısı ve saatiyle), ulaştı, okundu. Henüz ulaşmamış bir mesajın zaman çizelgesi kesik çizgili bir kum saati düğümüyle biter.
- **Yolda:** sohbet listesinde, henüz ulaşmamış mesajı olan konuşmalar için bir filtre vardır; bu konuşmalarda son mesajın önünde bir kum saati görünür.
- **Yazma alanı:** bir ekleme düğmesi, beş satıra kadar büyüyen hap şeklinde bir alan ve alan boşken mikrofon, metin yazılınca gönderme oku olan tek bir yuvarlak, vurgu renginde düğme. Paket sayısı alanın üstünde yalnızca metin birden fazla radyo paketi gerektirdiğinde ya da bir sorun olduğunda görünür.
- **Bağlantı özeti** (sohbet listesi): renkli bir nokta (herhangi bir bağlantı açıkken yeşil, hiçbiri açık değilken gri) ve "İnternet yok · yakında 3 cihaz" gibi tek bir cümle.
- **Ağ satırları:** simge kutusu, ad, sözcüklerle durum (çalışırken yeşil) ve bağlantının açılıp kapatılabildiği yerlerde bir anahtar. Radyosu kapalı olan bir bağlantı neyin açılması gerektiğini söyler, örneğin "Kullanmak için Bluetooth'u aç". Yeniden başlatma gerektiren değişiklikler tek bir yeniden başlatma satırı gösterir.
- **LoRa:** satırı bir pencere açar. Cihaz seçilmemişse pencere seçilebilecek Meshtastic ve MeshCore cihazlarını listeler: telefonlarda Bluetooth ile bulunanlar, bilgisayarlarda USB bağlantı noktaları ya da ağdaki bir adres; her türün dinlediği bağlantı noktasıyla birlikte. Cihaz seçilmişse şunları gösterir: cihazın adı, durumu ve türü; Meshtastic cihazında bölge (ayarlı değilse hata renginde, nasıl ayarlanacağıyla birlikte) ya da MeshCore cihazında telsiz ayarları; MeshCore cihazında boş kanal yuvası yoksa hata renginde bir not; CarryChat'in LoRa üzerinden en son ne zaman geldiği; son zamanlarda duyulan cihazlar (Meshtastic için üç saat, MeshCore için iki gün), ne kadar önce duyuldukları, atlama sayısı ve SNR değeriyle; ve cihazı kullanmayı bırakma düğmesi.
- **Reticulum:** satırı bir pencere açar. Düğüm seçilmemişse Reticulum'un ne olduğunu anlatır ve bir düğüm adresi alır. Düğüm seçilmişse adresi ve durumu, CarryChat'in Reticulum üzerinden en son ne zaman geldiğini, son altı saatte duyulan CarryChat düğümlerini (ne kadar önce duyulduklarını ve kaç atlama uzakta olduklarını), bu düğümün kendi hedef adresini (destination) ve Reticulum'u kullanmayı bırakma düğmesini gösterir.
- **Pencereler:** her iletişim kutusu alttan açılan bir penceredir: bir tutamaç, 20px başlık ve altta eylemleri vardır; telefonlarda tam genişlikte, masaüstünde 440px genişliğinde ve ortalanmış olarak açılır. Aşağı sürükleyerek, örtüye dokunarak, Escape ile ya da Android'in geri tuşuyla kapanır.
- **Boş durumlar** bir sonraki adımı tek cümleyle söyler.
- **Yakınımdakiler:** Kişiler sekmesinde bir "Yakınımdakilere görün" anahtarı. Açıkken Kişiler, yakındaki görünür kişileri bulundukları bağlantıyla birlikte listeler; Sohbetler de onları filtrelerin üstünde halkalı avatarlardan oluşan bir sıra olarak gösterir; birine dokunmak, onu önce kişi olarak ekleyip bir sohbet açar. Görünürlük karşılıklıdır ve varsayılan olarak kapalıdır.

## Yapılacaklar ve Yapılmayacaklar

- Yap: vurgu rengini yalnızca eylemler, giden balonlar, okunmamış sayıları ve etkin sekme için kullan.
- Yap: bekleyen mesajı kesik çizgili çerçeve ve kum saatiyle, sakin sözcüklerle göster.
- Yap: Phosphor simgelerini regular kalınlıkta kullan; etkin sekme için fill kalınlığını kullan.
- Yap: yazı tipini uygulamayla birlikte paketle; çalışma sırasında ağdan hiçbir şey yükleme.
- Yap: her iletişim kutusunu sürüklenerek kapatılabilen bir pencere olarak tut.
- Yapma: ekranların arkasına renk geçişi, parlama, bulanık cam ya da renkli arka plan ekleme.
- Yapma: ikinci bir vurgu rengi ya da ikinci bir yazı tipi ekleme.
- Yapma: bekleyen mesajı kırmızı ya da başarısız olmuş gibi gösterme.
- Yapma: satırlara, kartlara ya da balonlara gölge koyma.
- Yapma: etiketleri tamamen büyük harfle ya da harf aralığı açılmış küçük yazıyla yazma.

## Hareket

- **Yaklaşım:** minimal-functional (az ve işlevsel).
- **Yumuşatma (easing):** enter(ease-out) exit(ease-in) move(ease-in-out)
- **Süre:** micro(80ms) short(200ms) medium(300ms) long(500ms)
- **Ekranlar:** telefonda bir konuşma açılınca 220ms içinde sağdan kayarak gelir. Pencereler parmağı izler ve 300ms içinde yerine oturur.
- **Özel olarak tasarlanmış tek an:** bekleyen bir mesaj teslim edildiğinde kesik çizgili çerçevesi 200ms içinde dolarak düz, vurgu renginde balona dönüşür.
- Sistemde "hareketi azalt" açıkken her şey hareketsiz değişir.

## Karar Kaydı
| Tarih | Karar | Gerekçe |
|------|----------|-----------|
| 2026-10-26 | İlk tasarım sistemi oluşturuldu | Ürün bağlamından ve güncel mobil mesajlaşma tasarımlarının incelenmesinden /design-consultation ile oluşturuldu; oryantiring haritası yönü ve koyu, sıcak bir yön denendi, ardından telefonda tanıdık gelen açık ve sade bir arayüz lehine bırakıldı. |
