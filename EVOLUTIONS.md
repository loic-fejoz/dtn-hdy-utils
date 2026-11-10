# Liste des Évolutions Potentielles à Évaluer

Ce document recense les propositions d'évolutions fonctionnelles, architecturales et ergonomiques pour la suite logicielle DTN (`dtn-hdy-utils` et le serveur/BPA autonome `dtn-hdy-ham`). Chaque proposition fait l'objet d'une évaluation préliminaire détaillant le contexte, les bénéfices, les points d'attention et la recommandation d'implémentation.

---

## 1. Migration du moteur Markdown vers une bibliothèque éprouvée (`marked` + `DOMPurify`)

### Description et cas d'usage
Remplacer le parseur regex fait maison ([`markdown.ts`](file:///home/loic/projets/dtn-hdy-utils/dtn-hdy-ham/web/src/utils/markdown.ts)) par le moteur standard `marked` associé au santiseur HTML `DOMPurify` au sein de l'interface Preact (`dtn-hdy-ham/web`).

### Bénéfices attendus
- **Conformité stricte CommonMark / GitHub Flavored Markdown (GFM)** : Prise en charge native et sans faille des tableaux, listes imbriquées, blocs de citations (`>`), liens complexes avec parenthèses imbriquées et ponctuations attenantes.
- **Sécurité XSS renforcée et pérenne** : `DOMPurify` est activement maintenu et audité par des spécialistes en sécurité web, immunisant l'IHM contre les contournements sophistiqués (DOM clobbering, mXSS, entités malicieuses).

### Défis techniques et inconvénients
- **Poids du bundle web** : Ajoute environ 25 à 35 Ko (gzippé) au bundle de l'interface web embarquée.
- **Support des schémas d'URI personnalisés** : Nécessite de configurer explicitement `DOMPurify` (`ALLOWED_URI_REGEXP` ou hooks) pour autoriser le schéma `dtn://`, qui est bloqué par défaut.
- **Styling** : Nécessite un renderer `marked` personnalisé pour conserver les classes Tailwind CSS actuelles.

### Statut d'évaluation
> **Recommandé** si l'usage évolue vers l'échange de notes ou documentations structurées (tableaux, citations). L'implémentation regex actuelle reste suffisante pour du chat simple et des aperçus rapides.

---

## 2. Nommage dynamique et extension automatique au téléchargement de fichiers

### Description et cas d'usage
Lors du téléchargement d'un fichier reçu sans extension ou typé génériquement `application/octet-stream`, utiliser le type MIME auto-détecté par le backend pour générer un nom de fichier suggéré cohérent via l'en-tête HTTP `Content-Disposition: attachment; filename="..."` ou l'attribut `download` de l'IHM.

### Bénéfices attendus
- **Expérience utilisateur transparente** : Évite à l'opérateur d'ouvrir un fichier `.bin` ou sans extension et de devoir deviner manuellement l'application associée (ex: `.gpx`, `.sqlite`, `.svg`, `.pdf`).
- **Cohérence système** : L'OS hôte de l'utilisateur associe immédiatement l'application correcte au double-clic.

### Défis techniques et inconvénients
- Nécessite de maintenir une table inverse robuste MIME -> Extension par défaut dans `files_service.rs` et/ou côté frontend.
- Gestion des collisions si le nom d'origine contient déjà une extension différente de celle détectée.

### Statut d'évaluation
> **Priorité haute** : Faible coût de développement, valeur d'usage immédiate pour les opérateurs radio recevant des fichiers de terrain.

---

## 3. Prévisualisation cartographique intégrée des tracés géospatiaux (GPX, KML, GeoJSON)

### Description et cas d'usage
Intégrer une modale ou un volet cartographique léger (ex: basé sur [Leaflet](https://leafletjs.com/) avec tuiles locales mises en cache ou vectorielles) pour prévisualiser directement dans l'IHM de `dtn-hdy-ham` les fichiers de traces GPX, balises KML ou couches GeoJSON reçus par DTN.

### Bénéfices attendus
- **Pertinence opérationnelle terrain** : Permet aux équipes radio d'urgence ou de secours en zone blanche de visualiser immédiatement sur une carte des waypoints, traces de progression ou zones sinistrées sans logiciel tiers.
- **Intégration naturelle** avec la télémétrie SenML et les paquets d'observation géolocalisés.

### Défis techniques et inconvénients
- **Mode hors-ligne strict** : En zone blanche / secours, aucune tuile OpenStreetMap distante n'est accessible via Internet. Nécessite soit une gestion de fond de carte hors-ligne (MBTiles / raster embarqué léger), soit un affichage pur du tracé vectoriel (coordonnées relatives, profil altimétrique).
- Poids de la bibliothèque cartographique dans l'IHM.

### Statut d'évaluation
> **À évaluer** : Forte valeur métier en radioamateurisme et secours d'urgence, mais implique de définir la stratégie de fond de carte déconnecté.

---

## 4. Prévisualisation sécurisée des documents PDF dans l'IHM

### Description et cas d'usage
Permettre la consultation en lecture seule des documents PDF transmis via DTN (bulletins d'information, procédures d'urgence, météo) directement dans l'interface web de la station.

### Bénéfices attendus
- Consultation rapide des procédures et bulletins sans quitter la station de gestion DTN.
- Évite les allers-retours avec un lecteur externe sur tablette ou mobile.

### Défis techniques et inconvénients
- **Sécurité** : L'affichage direct via une balise `<iframe src="/api/files/download/...">` peut exposer à des vulnérabilités si le PDF contient du JavaScript embarqué. Nécessite une sandbox stricte (`sandbox="allow-scripts"` restreint) ou un rendu via PDF.js dans un canvas isolé.
- **Poids** : PDF.js complet est volumineux (~300 Ko gzippé) pour une station embarquée basse consommation (Raspberry Pi).

### Statut d'évaluation
> **À évaluer** : Tester la faisabilité d'un simple `iframe` sandboxed natif au navigateur avant d'envisager une bibliothèque dédiée.

---

## 5. Inspection et exploration du contenu des archives (ZIP, TAR, GZ)

### Description et cas d'usage
Afficher dans le volet détail d'un fichier archive la liste des fichiers contenus (arborescence, tailles individuelles) sans exiger le téléchargement et l'extraction locale préalable.

### Bénéfices attendus
- Permet à l'opérateur de vérifier le contenu d'un paquet de fichiers avant de décider de le distribuer ou de le décompresser.
- Évite les manipulations manuelles pour inspecter un panier `dtnbasket` ou un bundle groupé.

### Défis techniques et inconvénients
- Nécessite d'inspecter l'en-tête de l'archive côté backend Rust (`zip` ou `tar`) de manière sécurisée contre les "zip bombs" (limite sur le nombre d'entrées inspectées et profondeur d'arborescence).
- Ajoute de légères dépendances de parsing d'en-têtes d'archives côté serveur.

### Statut d'évaluation
> **Moyenne priorité** : Très utile pour la transparence opérationnelle, simple à implémenter côté Rust de façon bornée.

---

## 6. Barre d'outils d'assistance à la saisie Markdown dans le Chat

### Description et cas d'usage
Ajouter au-dessus du champ de texte du Chat ([`ChatView.tsx`](file:///home/loic/projets/dtn-hdy-utils/dtn-hdy-ham/web/src/components/ChatView.tsx)) une mini-barre d'outils ergonomique proposant des raccourcis : Gras (`**`), Italique (`*`), Code (` ` `), Lien Markdown (`[nom](url)`), et insertion rapide d'un identifiant EID (`dtn://...`).

### Bénéfices attendus
- Réduit les erreurs de frappe de syntaxe Markdown sur mobile ou écran tactile de terrain.
- Uniformise la mise en forme des messages sans obliger l'utilisateur à mémoriser la syntaxe.

### Défis techniques et inconvénients
- Ergonomie sur petits écrans (tablette/smartphone en PWA) : doit rester discret et ne pas masquer l'historique du chat lors de l'ouverture du clavier virtuel.

### Statut d'évaluation
> **Priorité haute** : Facile à intégrer dans le composant Preact existant, gain direct d'utilisabilité.

---

## 7. Compression transparente à l'émission pour liaisons radio lentes

### Description et cas d'usage
Proposer une option de compression transparente (ex: Zstandard ou Deflate) des payloads de bundles volumineux (fichiers texte, JSON, SenML, logs) avant transmission via des CLA radio à très faible débit (Mercury TNC, KISS HF/VHF, LTP).

### Bénéfices attendus
- Réduction significative du temps d'occupation spectral (canal radio) pour les flux textuels et de télémétrie.
- Réduction de la facture énergétique des émetteurs radio en émission prolongée.

### Défis techniques et inconvénients
- Doit être négocié ou signalé au niveau métadonnées du bundle ou type MIME pour que le nœud récepteur sache décompresser automatiquement.
- Inutile (voire contre-productif) sur les fichiers déjà compressés (JPEG, PNG, MP4, archives ZIP).

### Statut d'évaluation
> **À planifier** : Requiert une spécification au niveau protocole d'application DTN ou profil de CLA pour garantir l'interopérabilité.

---

## 8. Support des formulaires standardisés d'urgence (style Winlink / ICS-213)

### Description et cas d'usage
Permettre la saisie, l'envoi, la réception et la consultation de formulaires opérationnels d'urgence standardisés (très répandus dans l'écosystème Winlink Standard Forms, les réseaux ARES, RACES et ADRASEC de Sécurité Civile) :
- **Formulaires ICS (Incident Command System)** : ICS-213 (Message général), ICS-214 (Journal d'activité), ICS-205 (Plan radio).
- **Rapports de situation et de pointage** : Check-in / Check-out de station, rapport météo simplifié, évaluation rapide des dégâts (Field Situation Report).

Le système repose sur un modèle hybride sobre en bande passante :
1. Des gabarits de formulaires (HTML/JSON) pré-embarqués dans l'IHM web de la station.
2. Un échange radio ultra-compact : seules les données saisies (quelques centaines d'octets de JSON ou CBOR) transitent dans le payload du bundle DTN vers un endpoint dédié (ex: `dtn://station/forms` ou service Chat/Files).
3. Une reconstitution visuelle fidèle à la réception reprenant la présentation réglementaire du formulaire, avec options d'impression papier propre ou export PDF.

### Bénéfices attendus
- **Interopérabilité et adoption opérationnelle** : Les services de secours et préfectures utilisent le cadre ICS. L'intégration directe dans `dtn-hdy-ham` positionne la solution comme une alternative résiliente et store-and-forward face à Winlink.
- **Sobriété spectrale optimale** : Contrairement à l'envoi d'un PDF ou d'une capture d'écran, seuls les champs utiles sont véhiculés sur les liaisons HF/VHF (gain d'un facteur 50 à 100 sur le temps d'émission).
- **Validation et guidage** : Champs obligatoires contrôlés, coordonnées GPS issues de la station, horodatage UTC certifié.
- **Authentification BPSec** : Possibilité de signer cryptographiquement le formulaire avec la clé de la station (via le module BPSec/BIB déjà intégré dans `dtn-hdy-ham`) pour garantir l'inviolabilité des ordres de mission et rapports.

### Défis techniques et inconvénients
- **Format de données (Inter-op Winlink vs Natif CBOR/JSON)** : Évaluer s'il faut analyser/générer le dialecte XML standard de Winlink Express pour créer des passerelles directes, ou adopter un format JSON/CBOR standardisé plus léger.
- **Gestion du catalogue de gabarits** : Mécanisme de mise à jour des modèles sans recompiler toute l'application (ex: dossier de modèles personnalisables).
- **Affichage dégradé** : Si une station reçoit un formulaire dont elle ne possède pas le gabarit local, nécessité d'un rendu générique tableau "clé / valeur".

### Statut d'évaluation
> **Priorité haute / Stratégique** : Évolution majeure à forte valeur ajoutée pour les transmissions d'urgence et la communauté radioamateur.

---

## 9. Vérification formelle et garanties d'absence totale de panics (`no-panic` & `cargo-kani`)

### Description et cas d'usage
Dans un réseau tolérant aux délais (DTN) et particulièrement en contexte d'urgence ou d'infrastructure radio isolée, un crash de processus (`panic!`) sur un nœud intermédiaire ou une station autonome interrompt le stockage et le réacheminement de tous les paquets en transit. Au-delà des règles de linting statique (`clippy::unwrap_used` et `clippy::panic`), il s'agit d'intégrer des outils de garantie mathématique et de link-time vérifiant l'absence absolue de panics sur les chemins d'exécution critiques :

1. **Option A : Attribut de link-time `#[no_panic]` (David Tolnay)** :
   * Permet d'annoter les fonctions critiques de bas niveau (parsers de paquets, décodeurs réseau, fonctions cryptographiques).
   * Fonctionnement : S'appuie sur les optimisations du compilateur LLVM en mode `--release` (dead code elimination). Si le compilateur ne parvient pas à prouver qu'un chemin de panique est inaccessible, l'édition de liens échoue immédiatement avec un symbole manquant (`__no_panic`).
   * *Cibles idéales* : Parsing des segments LTP (`hardy-ltp`), décodeur CBOR SenML (`senml/parser.rs`), désérialisation des métadonnées de paniers CoAP/CBOR (`dtnbasket`).

2. **Option B : Bounded Model Checking avec Kani (`cargo-kani`, AWS)** :
   * Vérificateur formel bit-précis basé sur des solveurs SMT/SAT (CBMC).
   * Fonctionnement : Explore symboliquement l'ensemble infini des combinaisons d'octets d'entrée sur un "harness" de test formel pour prouver mathématiquement :
     * Zéro déréférencement hors bornes (`slice out-of-bounds` sur les indices de trames).
     * Zéro débordement arithmétique (`integer overflow`).
     * Zéro panique implicite (`Option::unwrap()` ou `Result::unwrap()`).
   * *Cibles idéales* : Machine à états finis de réassemblage LTP (`SessionFsm`), calcul des claims de segments, moteur de vérification BPSec/BIB (`keystore.rs`).

3. **Option C : Analyse statique de graphe d'appels (`cargo-call-stack` / Inspection de symboles)** :
   * Avec `panic = "abort"` dans le profil de release, vérification que l'arbre d'appels des binaires compilés (`dtn-hdy-ham`, CLAs) ne référence pas les symboles de panique interne `core::panicking::panic` ou `rust_begin_unwind`.

### Bénéfices attendus
- **Résilience absolue sur liaisons radio bruitées** : Les trames corrompues ou fuzzées reçues par RF ne peuvent en aucun cas provoquer le crash du daemon.
- **Zéro régression silencieuse** : Tout ajout de code potentiel provoquant un panic est détecté soit à la compilation (link-time), soit par les solveurs formels en CI.
- **Zéro surcharge à l'exécution** : Contrairement aux défenses défensives dynamiques (catch_unwind), ces approches s'appuient sur l'élimination statique.

### Défis techniques et inconvénients
- **`#[no_panic]`** : Nécessite des builds optimisés (`--release`), car en mode debug sans optimisation LLVM, le compilateur ne simplifie pas les branches de vérification d'indices.
- **Kani** : Les solveurs SMT requièrent des temps d'analyse plus longs en CI (plusieurs dizaines de secondes par harness) et imposent des bornes explicites (*unwind bounds*) sur les boucles dynamiques.

### Statut d'évaluation
> **Recommandé** : Intégration progressive en commençant par `#[no_panic]` sur les fonctions pures de parsing binaire de `hardy-ltp` et `senml`, puis écriture de harness Kani sur la FSM de session LTP.

---

## 10. Surcharge de configuration par variables d'environnement (`DTN_HDY_HAM__*`)

### Description et cas d'usage
Conformément aux directives d'architecture de Hardy (`hardy/docs/design.md` §7), permettre la surcharge dynamique de toute clé de configuration par des variables d'environnement préfixées par `DTN_HDY_HAM__` avec séparateur double tiret bas `__` pour la hiérarchie imbriquée (ex: `DTN_HDY_HAM__CALLSIGN=F4XYZ`, `DTN_HDY_HAM__STORAGE__IN_MEMORY=true`, `DTN_HDY_HAM__WEB__ADDRESS="0.0.0.0:8088"`).

### Bénéfices attendus
- **Déploiements conteneurisés simplifiés (Docker, Podman, Kubernetes)** : Permet de configurer une station DTN radioamateur complète via un fichier `docker-compose.yml` ou des variables d'environnement système sans injecter ni monter de volume de fichier de configuration.
- **Intégration systemd** : Possibilité d'utiliser `EnvironmentFile=/etc/default/dtn-hdy-ham` dans l'unité systemd pour les déploiements d'infrastructure.
- **Homogénéité avec l'écosystème Hardy** : Aligne `dtn-hdy-ham` avec le comportement standard des binaires `hardy-bpa-server` et `hardy-tcpclv4-server`.

### Défis techniques et inconvénients
- Nécessite d'ajouter la source `config::Environment::with_prefix("DTN_HDY_HAM").separator("__")` dans le constructeur de configuration de `HamConfig::load`.
- Veiller à préserver la priorité : valeurs par défaut < fichier de configuration < variables d'environnement < arguments de ligne de commande (CLI).

### Statut d'évaluation
> **Priorité haute / Facile** : Très faible coût de mise en œuvre dans `HamConfig::load`, gain opérationnel immédiat pour les conteneurs et déploiements automatisés.

---

## 11. Intégration du CLA basé sur fichiers (`hardy-file-cla` / Sneakernet)

### Description et cas d'usage
Intégrer le crate officiel `hardy-file-cla` de Hardy au sein de `dtn-hdy-ham` afin de permettre l'échange et le relayage de bundles BPv7 à travers des répertoires du système de fichiers (dossiers entrants `inbox` et sortants `outbox`), idéaux pour le transport physique ("Sneakernet") sur clés USB, cartes SD ou dossiers partagés synchronisés (Syncthing, Nextcloud, rsync).

### Bénéfices attendus
- **Résilience absolue en blackout total** : Même sans aucune liaison radio HF/VHF disponible ni connectivité IP, deux stations peuvent échanger des messages, fichiers de secours et télémétrie par simple transfert physique d'une clé USB.
- **Passerelle hybride Air-Gap** : Permet à une station isolée non connectée à Internet de transférer des bundles vers une passerelle via un média amovible.
- **Standardisation** : Réutilise directement le composant officiel `hardy-file-cla` maintenu dans l'arborescence Hardy.

### Défis techniques et inconvénients
- Surveillance de répertoire (polling vs notifications inotify/kqueue).
- Gestion des permissions de fichiers sur médias amovibles (systèmes de fichiers FAT32/exFAT sans permissions POSIX).
- Prise en compte dans l'IHM web (affichage de l'état du CLA fichier et du nombre de paquets déposés/lus).

### Statut d'évaluation
> **Recommandé** : Forte pertinence pour les exercices de Sécurité Civile et réseaux d'urgence en zone sinistrée.

