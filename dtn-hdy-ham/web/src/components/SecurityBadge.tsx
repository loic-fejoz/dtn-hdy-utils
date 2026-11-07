import { h } from 'preact';

export interface SecurityBadgeProps {
  status?: string | null;
  compact?: boolean;
  showLabel?: boolean;
  className?: string;
}

export function SecurityBadge({
  status,
  compact = false,
  showLabel = true,
  className = '',
}: SecurityBadgeProps) {
  const norm = (status || 'unsigned').toLowerCase();

  let icon = '❓';
  let label = 'Non signé';
  let title = 'Bundle standard BPv7 non signé';
  let colorClasses =
    'bg-slate-100 text-slate-600 border-slate-200 dark:bg-slate-800 dark:text-slate-400 dark:border-slate-700';

  if (norm === 'valid') {
    icon = '🛡️';
    label = 'Authentifié';
    title = 'Signature BPSec BIB valide (HMAC-SHA256 - RFC 9103 / RFC 9172)';
    colorClasses =
      'bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-950/60 dark:text-emerald-300 dark:border-emerald-800';
  } else if (norm === 'invalid') {
    icon = '⚠️';
    label = 'Signature Invalide';
    title = 'Échec de vérification BPSec : contenu altéré ou clé secrète incorrecte !';
    colorClasses =
      'bg-rose-50 text-rose-700 border-rose-200 dark:bg-rose-950/60 dark:text-rose-300 dark:border-rose-800 font-semibold';
  } else if (norm === 'missing_key') {
    icon = '❓';
    label = 'Clé manquante';
    title = 'Bundle signé avec BPSec BIB, mais aucune clé correspondante configurée';
    colorClasses =
      'bg-slate-100 text-slate-600 border-slate-200 dark:bg-slate-800 dark:text-slate-400 dark:border-slate-700';
  }

  if (compact) {
    return (
      <span
        class={`inline-flex items-center justify-center cursor-help select-none ${className}`}
        title={title}
        role="img"
        aria-label={label}
      >
        <span class="text-xs">{icon}</span>
      </span>
    );
  }

  return (
    <span
      class={`inline-flex items-center space-x-1 px-2 py-0.5 text-[11px] rounded-full border cursor-help select-none ${colorClasses} ${className}`}
      title={title}
      role="status"
    >
      <span class="text-xs leading-none">{icon}</span>
      {showLabel && <span class="font-medium leading-none">{label}</span>}
    </span>
  );
}
