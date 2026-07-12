# Guide de déploiement de `hdy-stats` sur machine distante

Ce guide décrit les étapes pour compiler, transférer et configurer `hdy-stats` comme service système avec `systemd` (`systemctl`) sur une machine distante (serveur x86 ou nœud Pi-Star ARMv7).

---

## 🛠️ 1. Compilation pour la cible distante

Selon l'architecture de votre machine distante, compilez le binaire depuis votre poste de développement :

### Option A : Pour un serveur distant x86 (64-bit Linux)
```bash
# Compilation standard portable (recommandée pour éviter l'erreur SIGILL)
RUSTFLAGS="-Ctarget-cpu=x86-64" cargo build --release --bin hdy-stats
```
Le binaire généré sera situé dans `target/release/hdy-stats`.

> [!WARNING]
> Si votre configuration Cargo globale (`~/.cargo/config.toml`) contient `rustflags = ["-Ctarget-cpu=native", ...]`, le compilateur optimisera le code pour le processeur de votre machine de développement. Si vous copiez ensuite ce binaire sur un serveur distant doté d'un CPU différent (ou d'une machine virtuelle), le programme plantera immédiatement au démarrage avec l'erreur **`Instruction non permise` (Illegal instruction)**.
> Utiliser explicitement `RUSTFLAGS="-Ctarget-cpu=x86-64"` désactive ces optimisations spécifiques et génère un binaire x86_64 portable.

### Option B : Pour un Raspberry Pi / Pi-Star (ARMv7 32-bit)
Vous pouvez cross-compiler en utilisant le target Rust approprié :
1. Installez le target Rust :
   ```bash
   rustup target add armv7-unknown-linux-gnueabihf
   ```
2. Compilez le binaire :
   ```bash
   cargo build --release --target armv7-unknown-linux-gnueabihf --bin hdy-stats
   ```
Le binaire généré sera situé dans `target/armv7-unknown-linux-gnueabihf/release/hdy-stats`.

---

## 📦 2. Transfert sur la machine distante

Utilisez `scp` ou `rsync` pour copier le binaire compilé sur le nœud distant.

*Exemple pour Pi-Star (IP : `192.168.3.2`, utilisateur : `pi-star`) :*
```bash
# Si Pi-Star (ARMv7)
scp target/armv7-unknown-linux-gnueabihf/release/hdy-stats pi-star@192.168.3.2:/home/pi-star/

# Si serveur x86
scp target/release/hdy-stats user@serveur-distant:/home/user/
```

---

## ⚙️ 3. Configuration de `systemd` (Service Systemctl)

Sur la machine distante, connectez-vous en SSH et configurez le service systemd afin que `hdy-stats` démarre automatiquement au boot.

### Cas 1 : Déploiement sur Pi-Star (avec logs redirigés sur clé USB)
Conformément aux préconisations de Pi-Star (`PI-STAR-STEPS.md`), Hardy écrit ses logs dans `/mnt/usb-storage/hardy-data/hardy.log` pour éviter d'user la carte SD. `hdy-stats` doit donc lire directement ce fichier de log.

1. Créez le fichier de service `/etc/systemd/system/hdy-stats.service` :
   ```ini
   [Unit]
   Description=Hardy BPA Stats Collector & Responder
   After=hardy-bpa.service
   Requires=hardy-bpa.service

   [Service]
   Type=simple
   User=pi-star
   Group=pi-star
   ExecStart=/home/pi-star/hdy-stats --database /home/pi-star/hdy-stats.db --log-file /mnt/usb-storage/hardy-data/hardy.log -v
   Restart=always
   RestartSec=5

   [Install]
   WantedBy=multi-user.target
   ```

2. Activez et démarrez le service :
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable hdy-stats.service
   sudo systemctl start hdy-stats.service
   ```

---

### Cas 2 : Déploiement sur Serveur x86 Standard (avec journald et utilisateur dédié)
Sur un serveur classique, les logs de Hardy transitent directement par le démon système journald. `hdy-stats` s'y abonne alors de façon native sous un utilisateur système dédié (`hdy-stats`).

1. **Création de l'utilisateur système** (avec son propre répertoire de données pour la base SQLite) :
   ```bash
   sudo adduser --system --group --home /var/lib/hdy-stats hdy-stats
   ```
2. **Autorisation de lecture de journalctl** :
   Ajoutez l'utilisateur au groupe `systemd-journal` pour qu'il puisse interroger les logs de Hardy sans `sudo` :
   ```bash
   sudo usermod -aG systemd-journal hdy-stats
   ```
3. **Copie du binaire** :
   Copiez le binaire portable dans un répertoire système accessible en exécution (évitez `/home/` pour contourner les erreurs de droits d'accès `Permission denied`) :
   ```bash
   sudo cp target/release/hdy-stats /usr/local/bin/hdy-stats
   sudo chmod 755 /usr/local/bin/hdy-stats
   ```
4. **Création du service systemd** `/etc/systemd/system/hdy-stats.service` :
   ```ini
   [Unit]
   Description=Hardy BPA Stats Collector & Responder
   After=hardy-bpa.service
   Requires=hardy-bpa.service

   [Service]
   Type=simple
   User=hdy-stats
   Group=hdy-stats
   WorkingDirectory=/var/lib/hdy-stats
   ExecStart=/usr/local/bin/hdy-stats --database /var/lib/hdy-stats/hdy-stats.db --journald-unit hardy-bpa -v
   Restart=always
   RestartSec=5

   [Install]
   WantedBy=multi-user.target
   ```

5. **Activation et démarrage du service** :
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable hdy-stats.service
   sudo systemctl start hdy-stats.service
   ```

---

## 🔍 4. Commandes d'administration courantes

Une fois le service configuré sur la machine distante, vous pouvez utiliser les commandes standard de `systemd` :

- **Vérifier le statut du collecteur** :
  ```bash
  systemctl status hdy-stats.service
  ```
- **Consulter les logs de l'application** :
  ```bash
  journalctl -u hdy-stats.service -f --no-pager
  ```
- **Consulter les stats cumulées en console** :
  Vous pouvez lancer l'utilitaire manuellement sur la machine distante en mode consultation pour interroger sa base de données :
  ```bash
  /home/pi-star/hdy-stats --database /home/pi-star/hdy-stats.db --show
  ```
