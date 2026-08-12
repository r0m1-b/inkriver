use crate::feed::Feed;
use feed_rs::parser;

/// Builds the complete in-memory feed dataset used by unit tests.
pub(crate) fn mock_feeds() -> Vec<Feed> {
    vec![
        parse_mock_feed(
            SUBSTACK_ASTRONOMY_FEED,
            "carnet-du-ciel",
            crate::article::Source::Substack,
        ),
        parse_mock_feed(
            MEDIUM_BREAD_FEED,
            "le-pain-patient",
            crate::article::Source::Medium,
        ),
    ]
}

/// Parses an in-memory RSS fixture and converts it into the application feed model.
///
/// This helper intentionally panics when fixture data is invalid because malformed
/// static test data represents a programming error rather than a runtime failure.
fn parse_mock_feed(xml: &str, id: &str, source: crate::article::Source) -> Feed {
    let raw_feed = parser::parse(xml.as_bytes()).expect("the mock RSS feed must be valid");

    Feed::new(
        id.to_string(),
        raw_feed
            .title
            .expect("the mock feed must have a title")
            .content,
        raw_feed
            .links
            .first()
            .expect("the mock feed must have a link")
            .href
            .clone(),
        raw_feed
            .description
            .expect("the mock feed must have a description")
            .content,
        raw_feed.authors.first().map(|author| author.name.clone()),
        source,
        raw_feed.entries,
    )
}

const SUBSTACK_ASTRONOMY_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>Carnet du ciel — Substack</title>
    <link>https://carnet-du-ciel.example</link>
    <description>Conseils d'observation astronomique pour les nuits sans nuages.</description>

    <item>
      <guid>substack-astronomie-1</guid>
      <title>Repérer Jupiter sans télescope</title>
      <link>https://carnet-du-ciel.example/p/reperer-jupiter</link>
      <pubDate>Mon, 06 Jul 2026 20:00:00 +0000</pubDate>
      <description>Une méthode simple pour distinguer Jupiter des étoiles voisines.</description>
      <content:encoded><![CDATA[
        <p>Jupiter ressemble à une étoile très brillante, mais sa lumière scintille beaucoup moins.
        Commencez par regarder vers le sud-est en début de nuit, puis vérifiez sa position avec une
        carte du ciel. Une paire de jumelles suffit souvent pour apercevoir ses quatre lunes principales.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>substack-astronomie-2</guid>
      <title>Pourquoi la Lune paraît immense à l'horizon</title>
      <link>https://carnet-du-ciel.example/p/illusion-lune-horizon</link>
      <pubDate>Mon, 13 Jul 2026 20:00:00 +0000</pubDate>
      <description>L'impressionnante Lune d'été n'est pas réellement plus grande.</description>
      <content:encoded><![CDATA[
        <p>Lorsque la Lune se trouve près de l'horizon, notre cerveau la compare aux arbres et aux
        bâtiments. Cette comparaison crée une illusion de grandeur. Une photographie prise avec le
        même zoom montre pourtant que son diamètre apparent reste presque identique au fil de la nuit.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>substack-astronomie-3</guid>
      <title>Préparer une nuit d'observation des Perséides</title>
      <link>https://carnet-du-ciel.example/p/nuit-perseides</link>
      <pubDate>Mon, 20 Jul 2026 20:00:00 +0000</pubDate>
      <description>Choisir le bon lieu et le bon moment pour observer les étoiles filantes.</description>
      <content:encoded><![CDATA[
        <p>Éloignez-vous des lampadaires et installez-vous dans un endroit offrant une vue dégagée.
        Laissez vos yeux s'habituer à l'obscurité pendant vingt minutes. Aucun télescope n'est nécessaire :
        une chaise inclinée, une couverture chaude et un peu de patience constituent le meilleur équipement.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>substack-astronomie-4</guid>
      <title>Lire une carte du ciel pour la première fois</title>
      <link>https://carnet-du-ciel.example/p/lire-carte-du-ciel</link>
      <pubDate>Mon, 27 Jul 2026 20:00:00 +0000</pubDate>
      <description>Orienter correctement une carte céleste et reconnaître ses premiers repères.</description>
      <content:encoded><![CDATA[
        <p>Tenez la carte au-dessus de votre tête et placez le point cardinal observé vers le bas.
        Commencez par identifier la Grande Ourse, puis prolongez le bord de sa casserole pour trouver
        l'étoile Polaire. Ce premier axe rend le reste de la carte beaucoup plus facile à comprendre.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>substack-astronomie-5</guid>
      <title>Observer Saturne avec une petite lunette</title>
      <link>https://carnet-du-ciel.example/p/observer-saturne</link>
      <pubDate>Wed, 29 Jul 2026 20:00:00 +0000</pubDate>
      <description>Des réglages modestes suffisent pour découvrir les anneaux de Saturne.</description>
      <content:encoded><![CDATA[
        <p>Utilisez d'abord un faible grossissement afin de placer Saturne au centre de l'oculaire.
        Augmentez ensuite progressivement la puissance sans dépasser ce que permet la turbulence.
        Les anneaux apparaissent comme deux petites anses lumineuses autour d'un disque couleur crème.</p>
      ]]></content:encoded>
    </item>
  </channel>
</rss>
"#;

const MEDIUM_BREAD_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>Le pain patient — Medium</title>
    <link>https://medium.com/@le-pain-patient</link>
    <description>Comprendre et réussir le pain au levain dans une cuisine ordinaire.</description>

    <item>
      <guid>medium-pain-1</guid>
      <title>Réveiller un levain après une semaine au réfrigérateur</title>
      <link>https://medium.com/@le-pain-patient/reveiller-un-levain</link>
      <pubDate>Tue, 07 Jul 2026 07:30:00 +0000</pubDate>
      <description>Deux rafraîchis permettent généralement de retrouver un levain vigoureux.</description>
      <content:encoded><![CDATA[
        <p>Sortez le levain le matin et conservez seulement vingt grammes. Ajoutez la même quantité
        d'eau puis de farine. Lorsqu'il double de volume, répétez l'opération : son odeur doit devenir
        fruitée et sa surface se couvrir de bulles régulières avant de préparer la pâte.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>medium-pain-2</guid>
      <title>Le test de la fenêtre explique une pâte bien pétrie</title>
      <link>https://medium.com/@le-pain-patient/test-de-la-fenetre</link>
      <pubDate>Tue, 14 Jul 2026 07:30:00 +0000</pubDate>
      <description>Étirer un morceau de pâte révèle immédiatement la qualité du réseau de gluten.</description>
      <content:encoded><![CDATA[
        <p>Prélevez une noix de pâte et étirez-la doucement entre vos doigts. Si elle devient assez
        fine pour laisser passer la lumière sans se déchirer, le gluten est suffisamment développé.
        Si elle casse aussitôt, laissez-la reposer dix minutes avant de reprendre quelques rabats.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>medium-pain-3</guid>
      <title>Obtenir une croûte croustillante dans un four domestique</title>
      <link>https://medium.com/@le-pain-patient/croute-croustillante</link>
      <pubDate>Tue, 21 Jul 2026 07:30:00 +0000</pubDate>
      <description>La vapeur du début de cuisson aide le pain à gonfler avant de former sa croûte.</description>
      <content:encoded><![CDATA[
        <p>Préchauffez une cocotte en fonte avec le four, puis déposez-y le pâton et refermez le
        couvercle. La vapeur libérée par la pâte reste emprisonnée pendant les vingt premières minutes.
        Retirez ensuite le couvercle afin que la croûte colore et perde son humidité.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>medium-pain-4</guid>
      <title>Reconnaître une fermentation trop longue</title>
      <link>https://medium.com/@le-pain-patient/fermentation-trop-longue</link>
      <pubDate>Tue, 28 Jul 2026 07:30:00 +0000</pubDate>
      <description>Une pâte affaissée et très collante signale souvent une fermentation excessive.</description>
      <content:encoded><![CDATA[
        <p>Une pâte correctement fermentée garde une légère tension et reprend lentement sa forme
        après une pression du doigt. Lorsqu'elle ne résiste plus, s'étale et dégage une odeur fortement
        alcoolisée, façonnez-la délicatement et réduisez la durée de fermentation lors de la prochaine fournée.</p>
      ]]></content:encoded>
    </item>

    <item>
      <guid>medium-pain-5</guid>
      <title>Conserver un pain au levain pendant quatre jours</title>
      <link>https://medium.com/@le-pain-patient/conserver-pain-levain</link>
      <pubDate>Thu, 30 Jul 2026 07:30:00 +0000</pubDate>
      <description>Le tissu et la congélation préservent mieux le pain que le réfrigérateur.</description>
      <content:encoded><![CDATA[
        <p>Laissez refroidir le pain complètement puis enveloppez-le dans un linge propre, face coupée
        contre une planche. Pour une conservation plus longue, tranchez-le et congelez les portions.
        Quelques minutes au grille-pain leur rendront une mie souple et une croûte nette.</p>
      ]]></content:encoded>
    </item>
  </channel>
</rss>
"#;
