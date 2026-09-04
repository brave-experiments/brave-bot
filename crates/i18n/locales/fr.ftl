# French. Written against locales/en-US.ftl, which owns the set of messages and the name and
# kind of every argument.
#
# Glossary, so that one thing is called one thing throughout:
#
#   planner      le planificateur   the model holding the conversation
#   processor    le processeur      an isolated model handed slots and nothing else
#   turn         le tour            one round of work, from a prompt to a reply
#   trusted      fiable             content the planner is allowed to read
#   untrusted    non fiable         content it is not
#   to vouch     approuver          a person saying they have read something
#   workspace    l'espace de travail
#   transcript   la transcription
#   scroller     le défilement      the mode Ctrl-O opens over the transcript
#   token        le jeton
#
# What is deliberately left in English: the names of commands (`/model`, `/add-dir`), of
# environment variables (`$EDITOR`), of release channels, and the letters a question is
# answered with. Those are typed rather than read.


## Compter

count-turns = { $count ->
    [one] { $count } tour
   *[other] { $count } tours
    }


## Le démarrage, et les mots affichés avant qu'une interface existe

cli-tagline =
    bravebot { $version } : un agent polyvalent résistant à l'injection de prompt
cli-usage-heading = Utilisation :
cli-usage-interactive = Démarrer une session interactive
cli-usage-task = Exécuter une seule tâche
cli-usage-piped = ... avec une entrée redirigée, jamais fiable
cli-usage-resume = Reprendre une session dans ce répertoire
cli-usage-doctor = Vérifier la configuration et le confinement
cli-usage-import = Importer un abonnement Leo Premium

cli-keys-heading = Touches interactives :
cli-key-send = Envoyer
cli-key-audit = Afficher ou masquer le journal d'audit
cli-key-history = Revenir sur les invites envoyées
cli-key-scroll = Faire défiler la transcription
cli-key-jump = Aller au début ou au plus récent
cli-key-cancel = Annuler un tour en cours, vider la saisie, ou partir
cli-key-leave = Partir

cli-commands-heading = Commandes interactives :
cli-name-a-file = Inclure un fichier de l'espace de travail comme contexte fiable

cli-options-heading = Options :
cli-option-file = Inclure un fichier de l'espace de travail comme contexte (répétable)
cli-option-mode = turn (par défaut) décide étape par étape ; manifest planifie tout le déroulement d'abord
cli-option-print = Non interactif. Lit l'entrée redirigée comme contexte en quarantaine
cli-option-trace = Afficher le journal d'audit
cli-option-help = Afficher ce message
cli-option-version = Afficher la version


## Ce qu'une exécution en ligne de commande dit quand elle ne peut pas démarrer

cli-unknown-option = option inconnue : { $flag }
cli-file-needs-a-path = --file demande un chemin
cli-mode-needs-a-name = --mode demande l'un de : { $names }
cli-unexpected-argument = argument inattendu : { $argument }
cli-task-required = une tâche est requise
cli-configuration-problem = erreur de configuration : { $problem }
cli-workspace-problem = erreur d'espace de travail : { $problem }
cli-interface-problem = erreur d'interface : { $problem }
cli-directory-unknown = impossible de savoir de quel répertoire il s'agit
cli-no-such-session = aucune session { $id } dans ce répertoire
cli-piped-input-unreadable = avertissement : impossible de lire l'entrée redirigée : { $problem }
cli-piped-input-too-large =
    l'entrée redirigée dépasse { $limit } Mio. Écrivez-la dans un fichier et nommez celui-ci
    à la place


## Ce qu'une exécution unique dit à côté de la réponse

cli-notice = note : { $notice }
cli-model-used = modèle : { $model }
cli-something-was-refused =
    note : un contrôle de la politique a refusé quelque chose pendant ce tour
cli-resume-heading = Reprenez cette session avec :


## L'état de la configuration et du confinement
#
# Ces noms sont posés dans une colonne de dix caractères : au-delà, la valeur qu'ils nomment
# ne s'aligne plus sur les autres.

doctor-configuration-ok = configuration OK
doctor-endpoint = adresse
doctor-premium = premium
doctor-premium-absent = non configuré
doctor-key-id = id de clé
doctor-model = modèle
doctor-model-chosen = { $model } (choisi avec /model)
doctor-model-default = { $model } (par défaut)
doctor-key-name = clé
doctor-key = { $key } (jamais transmise)
doctor-backend = service
doctor-backend-bedrock = AWS Bedrock
doctor-backend-aichat = Brave Leo
doctor-region = région
doctor-profile = profil
doctor-profile-absent = identifiants par défaut
doctor-tiers = modèles
doctor-tiers-absent = aucun configuré (définir ANTHROPIC_DEFAULT_OPUS_MODEL)
doctor-settings = réglages
doctor-settings-names = { $names }
doctor-settings-absent = aucun settings.json
doctor-leo = leo
doctor-subscription =
    abonnement { $environment } importé, { $unspent } identifiants sur { $total } non dépensés
doctor-confinement = confinement { $level }
confinement-kernel = imposé par le noyau
confinement-partial = partiel
confinement-none = aucun
doctor-mechanisms = mécanismes
doctor-network-denial = refus réseau
doctor-kernel-enforced = imposé par le noyau
doctor-not-enforced = NON imposé
doctor-confinement-unavailable = confinement indisponible


## Importer un abonnement Leo Premium

leo-no-premium-endpoint =
    avertissement : cette version n'a pas d'adresse premium, les identifiants importés ne
    seront donc pas utilisés
leo-set-and-rebuild = définissez { $variable } et recompilez
leo-unknown-channel = canal inconnu : { $channel }
leo-expected-channel = attendu parmi : stable, beta, nightly, development
leo-forgotten = abonnement importé oublié
leo-looking = recherche d'un abonnement Leo dans Brave { $channel }
leo-found = abonnement { $environment } trouvé : { $order }
leo-registering = enregistrement de cette installation comme nouvel appareil
leo-stored =
    { $count } identifiants enregistrés dans { $path }, valables jusqu'au { $expiry }
leo-browser-untouched =
    les requêtes premium les utiliseront désormais ; les identifiants du navigateur n'ont
    pas été touchés

subscription-unusable =
    l'abonnement importé n'a pas pu être utilisé ({ $problem }) ; ce tour utilise donc
    l'offre gratuite


## Approuver un répertoire, demandé une fois quand une session démarre ailleurs

trust-directory-title = faire confiance à ce répertoire ?
trust-directory-question = Approuver
trust-directory-explained =
    Les fichiers d'ici seront lus comme fiables, et les modifications qui leur sont
    apportées ne vous seront pas montrées une par une. Répondez non si ce code n'est pas
    le vôtre.
trust-directory-regardless =
    Dans tous les cas, tout ce qui vient du web ou d'un fichier non fiable vous est encore
    montré avant d'être écrit.
trust-directory-yes = lui faire confiance
trust-directory-no = me demander à chaque écriture
quit = quitter


## Choisir un thème, un modèle, ou une session à reprendre

theme-picker-title = thèmes
theme-picker-keys = ↑↓ choisir  ·  Entrée valider  ·  Échap garder l'actuel
model-picker-heading = Choisir un modèle
model-picker-keys = ↑↓ pour choisir  ·  Entrée pour valider  ·  Échap pour garder l'actuel
picker-current = actuel
picker-premium = premium
picker-model-bedrock-profile = { $tier } (votre profil AWS { $profile })
picker-model-bedrock = { $tier } (votre compte AWS)
resume-heading = Reprendre une session
resume-search-placeholder = Rechercher…
resume-keys =
    ↑↓ pour choisir  ·  Entrée pour reprendre  ·  tapez pour rechercher  ·  Échap pour une
    nouvelle session
resume-nothing-matches = aucune correspondance
resume-manifest-run =
    c'était une exécution manifest, qui ne peut pas être reprise ; démarrez une nouvelle
    session


## Commun à toutes les questions que l'interface s'arrête pour poser

stop-the-turn = arrêter le tour
scroll-more = ↑↓ { $count } de plus
scroll-back = ↑↓ retour


## Approuver une écriture

write-title = approuver cette écriture ?
write-create = Créer
write-overwrite = Remplacer
write-edit = Modifier
write-tally = +{ $added } -{ $removed }
write-too-large-to-show =
    le changement est trop grand pour être montré : { $added } lignes en remplacent
    { $removed }
write-untrusted = non fiable : personne n'a lu ceci, et le modèle ne l'a jamais vu
write-unchanged = { $count ->
    [one] … { $count } ligne inchangée
   *[other] … { $count } lignes inchangées
    }
write-yes = l'écrire
write-no = ne rien changer


## Approuver une commande

run-title = exécuter ceci ?
run-verb = Exécuter
run-stages = { $count ->
    [one] { $count } étape
   *[other] { $count } étapes
    }
run-in-directory = dans { $directory }
run-not-sandboxed =
    ceci n'est pas isolé : l'exécution a les mêmes accès que votre propre shell
run-releases-private =
    vos propres données lui sont aussi fournies, et elles partent d'ici avec elle
run-always-explained = a : approuver cette commande exacte pour le reste de cette session
run-always-means-both = ce qui veut dire les deux :
run-always-runs-again = elle s'exécute de nouveau sans rien demander, effets de bord compris
run-always-output-trusted = ce qu'elle affiche est fiable, et le modèle le lit
run-always-exact-arguments = ces arguments seulement : git log ne couvrirait pas git push
run-private-not-remembered =
    une entrée privée est soumise à chaque fois, celle-ci ne peut donc pas être retenue
run-yes = l'exécuter
run-always = toujours
run-no = ne pas l'exécuter


## Laisser le modèle lire ce qu'une commande a affiché

output-title = laisser le modèle lire ceci ?
output-verb = Lire
output-lines = { $count ->
    [one] { $count } ligne
   *[other] { $count } lignes
    }
output-printed-by = affiché par { $command }
output-unseen =
    le modèle n'a pas vu ceci. L'approuver le met dans son contexte, et il agira dessus.
output-empty = (rien n'a été affiché)
output-yes = le laisser lire ceci
output-no = le garder pour vous


## Approuver un fichier en quarantaine

vouch-title = laisser le modèle lire ce fichier ?
vouch-verb = Approuver
vouch-explained =
    le modèle ne peut pas lire ce fichier, il travaille donc à l'aveugle dessus.
    L'approuver lui permet de le lire pour le reste de cette session, ici et à chaque
    lecture ultérieure.
vouch-yes = l'approuver
vouch-no = le laisser en quarantaine


## Compter ce qu'une session accumule

count-rules = { $count ->
    [one] { $count } règle
   *[other] { $count } règles
    }
count-commands = { $count ->
    [one] { $count } commande
   *[other] { $count } commandes
    }
count-tokens = { $count ->
    [one] { $count } jeton
   *[other] { $count } jetons
    }
count-tokens-thousands = { $thousands } k jetons
# Le français écrit une virgule entre un nombre entier et sa fraction.
number-decimal-separator = ,


## Ce que /status rapporte de la session

status-session = Session
status-session-untitled = sans titre, rien n'a encore été envoyé
status-session-id = Id de session
status-directory = Répertoire
status-directory-trusted = fiable
status-directory-untrusted = non fiable, chaque écriture vous est donc montrée
status-also-open = Aussi ouvert
status-added-directory = ajouté avec /add-dir
status-model = Modèle
status-model-chosen = choisi avec /model
status-model-default = la valeur par défaut configurée
status-theme = Thème
status-theme-chosen = choisi avec /theme
status-served = Répondu par
status-served-instead = servi à la place du modèle demandé
status-endpoint = Adresse
status-premium-available = premium disponible, rien encore envoyé
status-premium-in-use = premium, un jeton a été dépensé
status-premium-not-spent = offre gratuite : aucun abonnement utilisé
status-free-tier = offre gratuite seulement
status-confinement = Confinement
status-this-session = Cette session
status-time = Temps
status-time-inference = sur le modèle
status-time-tools = exécution des outils
status-time-stalled = en attente de vous
status-time-overhead = non attribué
status-trust = Confiance
status-nothing-vouched-for = rien d'approuvé
status-trusted = fiable
status-untrusted = non fiable
status-programs = Programmes
status-every-run-is-asked = chaque exécution vous est soumise
status-trusted-commands = Commandes fiables
status-trusted-commands-note = exécutées sans rien demander, et leur sortie est fiable
status-and-more = … et { $count } de plus

# Le français emprunte les trois premières abréviations telles quelles.
environment-local = local
environment-dev = dev
environment-prod = prod
environment-custom = personnalisé


## L'indicateur dessiné pendant qu'un tour tourne

elapsed-seconds = { $seconds } s
elapsed-minutes = { $minutes } min { $seconds } s
indicator-tokens-read = ↓ { $tokens } jetons
indicator-tokens-written = ↑ { $tokens }
tokens-thousands = { $thousands } k
tokens-millions = { $millions } M
turn-done = tour { $turn } terminé
turn-failed = tour { $turn } interrompu


## Reprendre une session qui tournait ailleurs, ou sur autre chose

session-reopen-failed = impossible de rouvrir { $directory } : { $problem }
session-branch-moved =
    cette session tournait sur { $was } ; cette copie de travail est sur { $now }
session-branch-gone =
    cette session tournait sur { $was } ; cette copie de travail n'est sur aucune branche
session-branch-new =
    cette session ne tournait sur aucune branche ; cette copie de travail est sur { $now }
session-build-differs = cette session tournait sous bravebot { $was } ; celle-ci est { $now }


## Thèmes

theme-follows-terminal = suit votre terminal, clair ou sombre


## Répondre à une question de l'agent

ask-title = l'agent pose une question
ask-title-numbered = l'agent pose une question ({ $at } sur { $total })
ask-own-words = Répondre avec mes propres mots
ask-more-options = … { $count } de plus, utilisez les flèches
ask-key-move = déplacer
ask-key-pick-any = cocher
ask-key-pick-one = choisir
ask-key-answer = répondre
ask-key-skip = passer
ask-key-skip-question = passer la question
ask-key-back-to-options = revenir aux options


## Confier la ligne à un éditeur

editor-none-configured =
    aucun éditeur trouvé : réglez $VISUAL ou $EDITOR sur celui que vous voulez
editor-scratch-unusable = le fichier à éditer n'a pas pu être utilisé : { $problem }
editor-named-but-missing =
    '{ $command }' est introuvable, et $VISUAL ou $EDITOR le nomme, rien d'autre n'a donc
    été essayé
editor-exited-badly =
    { $editor } s'est terminé avec le code { $code }, la ligne est donc inchangée
editor-was-stopped = { $editor } a été arrêté avant de finir, la ligne est donc inchangée
editor-would-not-start = { $editor } n'a pas démarré : { $problem }


## La transcription

input-placeholder = Demandez n'importe quoi à Brave Bot
quarantined-heading = non fiable · { $origin } · { $label }
transcript-more-lines = { $count ->
    [one] … { $count } ligne de plus
   *[other] … { $count } lignes de plus
    }
transcript-unchanged = { $count ->
    [one] … { $count } ligne inchangée
   *[other] … { $count } lignes inchangées
    }


## Relire la transcription

scroller-title = défilement
scroller-key-line = ligne haut/bas
scroller-key-half-page = demi-page
scroller-key-full-page = page entière   (aussi ctrl-f / ctrl-b)
scroller-key-ends = début / fin   (aussi home / end)
scroller-key-prompts = invite précédente / suivante
scroller-key-search = rechercher, correspondance suivante/précédente
scroller-key-editor = ouvrir la transcription dans $EDITOR
scroller-key-this-list = cette liste
scroller-key-close = fermer le défilement
scroller-searching = Entrée pour rechercher  ·  Échap pour abandonner
scroller-no-matches = aucune correspondance
scroller-match-of = { $at } sur { $total }
scroller-search-keys = n suivant  ·  N précédent  ·  Échap efface  ·  q ferme
scroller-rows-below = { $count ->
    [one] { $count } ligne en dessous
   *[other] { $count } lignes en dessous
    }
scroller-footer = défilement
scroller-footer-keys = q ferme  ·  ? touches
scroller-footer-search = / rechercher


## Ce qu'une ligne commençant par une barre oblique peut être

command-status = Décrire cette session, ce qu'elle peut toucher, et ce qu'elle a dépensé
command-model = Choisir avec quel modèle réfléchir
command-theme = Choisir quel thème habille l'interface
command-add-dir = Ouvrir un autre répertoire, et l'approuver pour cette session
command-rename = Appeler cette conversation autrement
command-compact = Résumer la conversation jusqu'ici, en gardant la partie récente
command-clear = Démarrer une nouvelle session ici, celle-ci restant reprenable
command-exit = Partir


## Ce que la session répond

session-resumed = session reprise : { $title }
session-renamed = renommée en { $title }
session-rename-needs-a-name = /rename demande un nom, comme /rename le bug de l'analyseur
session-rename-needs-something = /rename demande un nom qui contienne quelque chose
session-cleared = effacée : une nouvelle session, la précédente restant reprenable
session-add-dir-needs-a-path = /add-dir demande un répertoire, comme /add-dir ~/notes
session-directory-added = { $directory } ajouté, et approuvé pour cette session
session-directory-not-added = impossible d'ajouter { $directory } : { $problem }
session-using-model = utilise { $model }
session-signing-in =
    connexion à AWS ; suivez les instructions ci-dessous, cela reprend une fois terminé
session-context-budget = compactage au-delà de { $budget } jetons, selon ce que ce modèle annonce
session-models-unavailable = impossible de lister les modèles : { $problem }
session-theme-set = thème { $theme }
session-no-such-theme = aucun thème nommé { $theme } ; essayez /theme pour la liste
session-trusting = { $directory } approuvé
session-trusting-as-left = { $directory } approuvé (comme cette session l'avait laissé)
session-not-trusting =
    ce répertoire n'est pas approuvé ; chaque écriture vous sera montrée
session-vouched-for = { $path } approuvé pour cette session
session-answered-already = déjà répondu : { $question }
session-something-was-refused =
    un contrôle de la politique a refusé quelque chose pendant ce tour
session-model-substituted =
    { $asked } n'a pas été servi : l'adresse a répondu avec { $served }. Lancez
    `bravebot doctor` si un abonnement était attendu.
session-error = erreur : { $problem }
session-no-output = aucune sortie


## Coller, déposer et joindre

paste-arrived-empty =
    ce collage est arrivé vide : le terminal ne transmet que du texte, une image demande
    donc ctrl-v
paste-not-a-command = une image n'est pas une commande : quittez le mode shell pour en coller une
paste-too-large = cette image fait { $size }, et un collage en porte au plus { $limit }
paste-nothing-on-clipboard = il n'y a rien à coller dans le presse-papiers
paste-folded = { $lines ->
    [one] [Texte collé #{ $number } +{ $lines } ligne]
   *[other] [Texte collé #{ $number } +{ $lines } lignes]
    }
megabytes = { $size } Mo


## Exécuter une commande que la personne a tapée

command-thread-stopped = le fil de la commande s'est arrêté de façon inattendue
command-reported-a-failure = la commande a signalé un échec


## Raccourcir une longue conversation

compact-uninterruptible = un résumé ne peut pas être interrompu ; il tient en une requête
compact-ended-unexpectedly = le résumé s'est terminé de façon inattendue
compact-done =
    { $summarised } messages antérieurs résumés, les { $kept } derniers gardés tels quels
compact-nothing-to-do = il n'y a encore rien à résumer
compact-failed = la conversation n'a pas pu être résumée : { $problem }
turn-ended-unexpectedly = le tour s'est terminé de façon inattendue


## L'écran d'accueil

opening-confinement = confinement { $level }
opening-invitation = Posez une question sur cet espace de travail.


## Ce qu'un tour a fait, dans les mots qui ouvrent une ligne de transcription

verb-read-file = Lire
verb-list-files = Lister
verb-search = Chercher
verb-write-file = Écrire
verb-edit-file = Modifier
verb-todo-write = Planifier
verb-spawn-processor = Processeur isolé
verb-vet-content = Contrôler
verb-load-skill = Compétence
verb-ask-user = Demander
verb-run = Exécuter
verb-read-output = Lire la sortie
verb-unknown = Outil


## Où a atterri ce qu'un appel a produit, dit en fin de ligne

landed-in-the-planner = lu dans le contexte du planificateur
landed-quarantined =
    pas dans le contexte du planificateur ; seul un processeur isolé peut être envoyé le lire
landed-reserved = lu par rien : seul son nom est connu
reach-not-the-planner =
    pas dans le contexte du planificateur ; un processeur peut être envoyé le lire
reach-no-model = dans le contexte d'aucun modèle : rien ne peut être envoyé lire ceci
