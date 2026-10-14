#!/usr/bin/env bash
#
# Общее для хуков: кто вправе быть автором и какие строки про ИИ недопустимы.
#
# Зачем. Адрес в коммите решает, кто попадёт в Contributors на GitHub.
# Трейлер Co-Authored-By на noreply@anthropic.com добавляет туда аккаунт
# claude, чужой адрес - чужой аккаунт. В Contributors должен стоять только
# DotNotFact. Строку «Generated with Claude Code» владелец не хочет нигде.
#
# Почему проверка здесь, а не только в ~/.githooks: локальный
# core.hooksPath=.githooks перебивает глобальный, и глобальные проверки
# авторства в этом репозитории не срабатывают.
#
# Обход проверки автора - IDENTITY_FORCE=1, и только он. Не --no-verify на
# пуше: тот снимает заодно паузу между пушами.

IDENTITY_CANON="140886710+DotNotFact@users.noreply.github.com"

# Для истории (pre-push): kilyatti@gmail.com привязан к тому же аккаунту,
# noreply@github.com - коммиттер слияний через веб GitHub, в авторы не попадает.
identity_allowed() {
  case "${1:-}" in
    "$IDENTITY_CANON"|kilyatti@gmail.com|noreply@github.com) return 0 ;;
    *) return 1 ;;
  esac
}

# Строки про ИИ в тексте сообщения. Узко намеренно: «сгенерировано командой
# node tools/status.mjs» - законная фраза, а «Generated with Claude Code» нет.
IDENTITY_AI_TRAILER='^co-authored-by:.*(claude|anthropic|openai|chatgpt|copilot|gemini|cursor|codex)'
IDENTITY_AI_BANNER='(generated|created|written) (with|by) .*(claude|chatgpt|copilot|gemini|cursor|codex|\bai\b)'

# Печатает строки сообщения со стдина, которые нарушают правило; код 0 - чисто.
# Всё ниже «>8» (дифф от git commit -v) не смотрится. Строки с «#» смотрятся:
# у git commit -m они остаются в коммите.
identity_ai_lines() {
  perl -CSD -ne '
    last if /^# -+ >8 -+$/;
    if (/'"$IDENTITY_AI_TRAILER"'/i || /'"$IDENTITY_AI_BANNER"'/i || /\x{1F916}/) { print "    $_"; $bad = 1 }
    END { exit($bad ? 1 : 0) }'
}

identity_email_of() {
  git var "$1" 2>/dev/null | sed -E 's/.*<([^>]*)>.*/\1/'
}
