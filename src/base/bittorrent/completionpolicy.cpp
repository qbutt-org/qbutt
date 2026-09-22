/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "completionpolicy.h"

#include <algorithm>
#include <cmath>
#include <cstdlib>

#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QSaveFile>

#ifdef Q_OS_WIN
#include <qt_windows.h>
#include <io.h>
#endif

#include "base/global.h"
#include "base/logger.h"
#include "base/path.h"
#include "base/preferences.h"
#include "base/profile.h"
#include "downloadpriority.h"
#include "sessionimpl.h"
#include "torrentimpl.h"

namespace
{
    constexpr qint64 StoreSizeLimit = 16 * 1024 * 1024;

#ifdef QBUTT_COMPLETION_FAULTS
    void fault(const char *point)
    {
        if (qEnvironmentVariable("QBUTT_COMPLETION_FAULT") == QLatin1StringView(point))
        {
#ifdef Q_OS_WIN
            TerminateProcess(GetCurrentProcess(), 198);
#else
            std::_Exit(198);
#endif
        }
    }
#endif

    QString policyPath(const QString &name)
    {
        return (specialFolderLocation(SpecialFolder::Data) / Path(u"completion/" + name + u".json")).toString();
    }

    QString save(const QString &path, const QJsonDocument &document)
    {
        QDir().mkpath(QFileInfo(path).absolutePath());
        QSaveFile file(path);
        const QByteArray bytes = document.toJson(QJsonDocument::Compact);
        if (bytes.size() > StoreSizeLimit)
            return BitTorrent::CompletionPolicy::tr("Completion journal capacity reached; automatic actions are held.");
        if (!file.open(QIODevice::WriteOnly) || (file.write(bytes) != bytes.size()) || !file.flush())
            return file.errorString();
#ifdef Q_OS_WIN
        if (!FlushFileBuffers(reinterpret_cast<HANDLE>(_get_osfhandle(file.handle()))))
            return BitTorrent::CompletionPolicy::tr("Cannot flush completion policy data to disk.");
#endif
        if (!file.commit())
            return file.errorString();
        return {};
    }
}

QString BitTorrent::CompletionPolicy::validateConfiguration(const QJsonObject &configuration)
{
    if (!configuration[u"enabled"_s].isBool() || !configuration[u"allow_delete_data"_s].isBool()
        || !configuration[u"rules"_s].isArray() || (configuration[u"rules"_s].toArray().size() > 64))
        return tr("Provide enabled, allow_delete_data and at most 64 rules.");
    for (const QString &key : configuration.keys())
    {
        if ((key != u"enabled") && (key != u"allow_delete_data") && (key != u"rules"))
            return tr("Unknown completion setting: %1").arg(key);
    }
    QSet<QString> ids;
    for (const QJsonValue &value : configuration[u"rules"_s].toArray())
    {
        const QJsonObject rule = value.toObject();
        const QString id = rule[u"id"_s].toString();
        if (id.isEmpty() || (id.size() > 128) || ids.contains(id) || !rule[u"enabled"_s].isBool()
            || !rule[u"match"_s].isObject() || !rule[u"actions"_s].isArray())
            return tr("Every rule needs a unique ID, enabled flag, match object and actions.");
        ids.insert(id);
        for (const QString &key : rule.keys())
        {
            if ((key != u"id") && (key != u"enabled") && (key != u"match") && (key != u"actions") && (key != u"on"))
                return tr("Unknown rule setting: %1").arg(key);
        }
        if (rule.contains(u"on"_s) && (rule[u"on"_s].toString() != u"wanted_files_committed"))
            return tr("Rules run on wanted_files_committed.");
        const QJsonObject match = rule[u"match"_s].toObject();
        for (auto it = match.begin(); it != match.end(); ++it)
        {
            if (it.key() == u"category")
            {
                if (!it.value().isString() || (it.value().toString().size() > 1024))
                    return tr("Category must be a string.");
            }
            else if (it.key() == u"tags")
            {
                if (!it.value().isArray() || (it.value().toArray().size() > 64))
                    return tr("Tags must be an array of at most 64 names.");
                for (const QJsonValue &tag : it.value().toArray())
                {
                    if (!tag.isString() || tag.toString().isEmpty() || (tag.toString().size() > 256))
                        return tr("Tag names must be nonempty strings.");
                }
            }
            else if ((it.key() == u"min_ratio") || (it.key() == u"min_seeding_seconds"))
            {
                const double number = it.value().toDouble(-1);
                if (!it.value().isDouble() || !std::isfinite(number) || (number < 0)
                    || (number > 1e12) || ((it.key() == u"min_seeding_seconds") && (std::floor(number) != number)))
                    return tr("Ratio and seeding time thresholds must be finite nonnegative numbers.");
            }
            else
                return tr("Unknown rule condition: %1").arg(it.key());
        }
        int terminal = 0;
        QSet<QString> actions;
        for (const QJsonValue &actionValue : rule[u"actions"_s].toArray())
        {
            const QString action = actionValue.toString();
            if (actions.contains(action))
                return tr("Duplicate rule action.");
            actions.insert(action);
            if ((action == u"stop") || (action == u"remove_torrent") || (action == u"delete_data"))
                ++terminal;
            else if (action != u"notify")
                return tr("Unknown rule action: %1").arg(action);
            if ((action == u"delete_data") && !configuration[u"allow_delete_data"_s].toBool())
                return tr("Deleting payload requires the separate allow_delete_data setting.");
        }
        if (actions.isEmpty() || (terminal > 1))
            return tr("Choose actions with at most one of stop, remove_torrent or delete_data.");
    }
    return {};
}

using namespace BitTorrent;

CompletionPolicy::CompletionPolicy(SessionImpl *session)
    : QObject(session)
    , m_session(session)
    , m_configuration {{u"enabled"_s, false}, {u"allow_delete_data"_s, false}, {u"rules"_s, QJsonArray()}}
{
    for (const QString &name : {u"rules"_s, u"journal"_s})
    {
        QFile file(policyPath(name));
        if (!file.exists())
            continue;
        if (!file.open(QIODevice::ReadOnly) || (file.size() > StoreSizeLimit))
        {
            m_error = tr("Cannot read completion policy %1.").arg(name);
            break;
        }
        const QJsonDocument document = QJsonDocument::fromJson(file.readAll());
        if (name == u"rules")
        {
            m_error = validateConfiguration(document.object());
            if (m_error.isEmpty())
                m_configuration = document.object();
        }
        else if (document.isArray())
        {
            m_journal = document.array();
            for (const QJsonValue &entry : m_journal)
            {
                const QString key = entry.toObject()[u"key"_s].toString();
                const QJsonObject record = entry.toObject();
                const QString status = record[u"status"_s].toString();
                bool validTime = false;
                record[u"added_at"_s].toString().toLongLong(&validTime);
                if ((key.size() != 64) || (QByteArray::fromHex(key.toLatin1()).toHex() != key.toLatin1())
                    || !validTime || !record[u"reason"_s].isObject() || !record[u"actions"_s].isArray()
                    || ((status != u"claimed") && (status != u"dispatched") && (status != u"blocked")))
                    m_error = tr("Invalid completion policy journal entry.");
                m_executed.insert(key);
            }
        }
        else
            m_error = tr("Invalid completion policy journal.");
        if (!m_error.isEmpty())
            break;
    }
    connect(&m_watcher, &QFutureWatcher<bool>::finished, this, [this]
    {
        if (!m_current || m_watcher.isCanceled() || !m_watcher.result())
        {
            finish(false);
            return;
        }
        if (!m_persisting)
        {
            m_persisting = true;
            if (m_acknowledging)
                m_current->setCompletionPolicyPreview(false);
            m_watcher.setFuture(m_session->persistStoppedTorrent(m_current, m_current->actualStorageLocation()));
            return;
        }
        finish(true);
    });
    connect(session, &Session::restored, this, [this]
    {
        if (!m_error.isEmpty())
        {
            for (Torrent *torrent : m_session->torrents())
                torrent->setCompletionPolicyPreview(true);
        }
        for (const QJsonValue &value : m_journal)
        {
            const QJsonObject entry = value.toObject();
            const QJsonArray actions = entry[u"actions"_s].toArray();
            if ((entry[u"status"_s].toString() == u"dispatched")
                && !actions.contains(u"remove_torrent"_s) && !actions.contains(u"delete_data"_s))
                continue;
            Torrent *torrent = m_session->getTorrent(TorrentID::fromString(entry[u"hash"_s].toString()));
            if (torrent && (QString::number(torrent->addedTime().toMSecsSinceEpoch()) == entry[u"added_at"_s].toString()))
                torrent->setCompletionPolicyPreview(true);
        }
    });
    connect(session, &Session::torrentAboutToBeRemoved, this, [this](Torrent *torrent)
    {
        m_pending.remove(torrent->id());
    });
    m_timer.setInterval(1000);
    connect(&m_timer, &QTimer::timeout, this, &CompletionPolicy::process);
    m_timer.start();
}

CompletionPolicy::~CompletionPolicy()
{
    if (m_current)
        m_current->endCompletion(false);
}

QJsonObject CompletionPolicy::configuration() const
{
    QJsonObject result = m_configuration;
    if (!m_error.isEmpty())
        result[u"error"_s] = m_error;
    return result;
}

QString CompletionPolicy::configure(const QJsonObject &configuration)
{
    if (isBusy())
        return tr("Wait for the current completion action to finish.");
    if (const QString error = validateConfiguration(configuration); !error.isEmpty())
        return error;
    if (!m_error.isEmpty())
        return m_error;
    if (const QString error = save(policyPath(u"rules"_s), QJsonDocument(configuration)); !error.isEmpty())
        return error;
    m_configuration = configuration;
    emit changed();
    return {};
}

QJsonArray CompletionPolicy::evaluate(const TorrentImpl *torrent, const QJsonObject &configuration) const
{
    QJsonArray result;
    if (!m_error.isEmpty())
        return result;
    QStringList tags;
    for (const Tag &tag : torrent->tags())
        tags.append(tag.toString());
    tags.sort();
    QString selected;
    for (const DownloadPriority priority : torrent->filePriorities())
        selected += (priority == DownloadPriority::Ignored) ? u'0' : u'1';
    for (const QJsonValue &value : configuration[u"rules"_s].toArray())
    {
        const QJsonObject rule = value.toObject();
        const QJsonObject match = rule[u"match"_s].toObject();
        if (!rule[u"enabled"_s].toBool()
            || (match.contains(u"category"_s) && (match[u"category"_s].toString() != torrent->category()))
            || (torrent->realRatio() < match[u"min_ratio"_s].toDouble())
            || (torrent->finishedTime() < match[u"min_seeding_seconds"_s].toDouble()))
            continue;
        bool matchesTags = true;
        for (const QJsonValue &tag : match[u"tags"_s].toArray())
            matchesTags &= tags.contains(tag.toString());
        if (!matchesTags)
            continue;
        const QJsonArray actions = rule[u"actions"_s].toArray();
        // A rule ID denotes one durable action per torrent incarnation and
        // wanted-file selection. Editing its conditions does not replay it.
        const QJsonArray identity {torrent->id().toString(), QString::number(torrent->addedTime().toMSecsSinceEpoch())
            , selected, rule[u"id"_s]};
        const QString key = QString::fromLatin1(QCryptographicHash::hash(QJsonDocument(identity).toJson(QJsonDocument::Compact)
            , QCryptographicHash::Sha256).toHex());
        result.append(QJsonObject {{u"key"_s, key}, {u"hash"_s, torrent->id().toString()}, {u"name"_s, torrent->name()}
            , {u"added_at"_s, QString::number(torrent->addedTime().toMSecsSinceEpoch())}
            , {u"rule"_s, rule[u"id"_s]}, {u"actions"_s, actions}, {u"already_claimed"_s, m_executed.contains(key)}
            , {u"reason"_s, QJsonObject {{u"category"_s, torrent->category()}, {u"tags"_s, QJsonArray::fromStringList(tags)}
                , {u"ratio"_s, torrent->realRatio()}, {u"seeding_seconds"_s, torrent->finishedTime()}, {u"wanted"_s, selected}
                , {u"destination"_s, torrent->actualStorageLocation().toString()}}}});
        if (actions.contains(u"stop"_s) || actions.contains(u"remove_torrent"_s) || actions.contains(u"delete_data"_s))
            break;
    }
    return result;
}

QJsonArray CompletionPolicy::preview(const QJsonObject &configuration) const
{
    QJsonArray result;
    for (Torrent *item : m_session->torrents())
    {
        const auto *torrent = static_cast<TorrentImpl *>(item);
        result.append(QJsonObject {{u"hash"_s, torrent->id().toString()}, {u"name"_s, torrent->name()}
            , {u"preview_required"_s, torrent->isCompletionPolicyPreview()}, {u"ready"_s, torrent->isReadyForCompletion() && !m_pending.contains(torrent->id())}
            , {u"recheck_paused"_s, torrent->isChecking() && torrent->isStopped()}
            , {u"rules"_s, evaluate(torrent, configuration.isEmpty() ? m_configuration : configuration)}});
    }
    return result;
}

QJsonArray CompletionPolicy::journal() const
{
    return m_journal;
}

QString CompletionPolicy::acknowledgePreview(const TorrentID &id)
{
    if (!m_error.isEmpty())
        return m_error;
    auto *torrent = static_cast<TorrentImpl *>(m_session->getTorrent(id));
    if (!torrent || !torrent->isCompletionPolicyPreview())
        return tr("This torrent does not require a completion policy preview.");
    if (isBusy() || !torrent->isStopped() || !torrent->beginCompletion(true))
        return tr("Stop the torrent and wait for its file operations before accepting the preview.");
    m_current = torrent;
    m_acknowledging = true;
    m_persisting = false;
    m_watcher.setFuture(m_session->drainTorrentDisk(torrent));
    emit changed();
    return {};
}

void CompletionPolicy::enqueue(TorrentImpl *torrent, const bool nativeEvent)
{
    m_pending[torrent->id()] = m_pending.value(torrent->id()) || nativeEvent;
}

bool CompletionPolicy::hasPendingEvents() const
{
    return std::ranges::any_of(m_pending, [](const bool nativeEvent) { return nativeEvent; });
}

bool CompletionPolicy::isBusy() const
{
    return !m_current.isNull();
}

bool CompletionPolicy::hasError() const
{
    return !m_error.isEmpty();
}

void CompletionPolicy::process()
{
    if (isBusy() || hasError() || !m_session->isRestored())
        return;
    if (m_notifySession && m_session->canRunCompletionAction())
    {
        m_notifySession = false;
        emit allWantedFilesCommitted();
    }
    for (Torrent *item : m_session->torrents())
    {
        auto *torrent = static_cast<TorrentImpl *>(item);
        if (torrent->isCompletionPolicyPreview() || !torrent->isReadyForCompletion())
            continue;
        const QJsonArray matches = m_configuration[u"enabled"_s].toBool() ? evaluate(torrent, m_configuration) : QJsonArray();
        const bool actionable = std::ranges::any_of(matches, [](const QJsonValue &entry)
        {
            return !entry.toObject()[u"already_claimed"_s].toBool();
        });
        if ((!m_pending.contains(torrent->id()) && !actionable) || !torrent->beginCompletion())
            continue;
        m_current = torrent;
        m_nativeEvent = m_pending.take(torrent->id());
        m_acknowledging = false;
        m_persisting = false;
        m_watcher.setFuture(m_session->drainTorrentDisk(torrent));
        emit changed();
        return;
    }
}

QString CompletionPolicy::saveJournal(const QJsonArray &entries)
{
    const QString error = save(policyPath(u"journal"_s), QJsonDocument(entries));
    if (error.isEmpty())
        m_journal = entries;
    return error;
}

void CompletionPolicy::finish(bool success)
{
    QPointer<TorrentImpl> torrent = m_current;
    const bool notifyFinished = m_nativeEvent;
    m_nativeEvent = false;
    if (!torrent)
    {
        m_current.clear();
        return;
    }
    if (m_acknowledging)
    {
#ifdef QBUTT_COMPLETION_FAULTS
        if (success)
            fault("acknowledged");
#endif
        if (!success)
            torrent->setCompletionPolicyPreview(true);
        torrent->endCompletion(false);
        m_current.clear();
        emit changed();
        return;
    }
    QJsonArray claims;
    if (success && !torrent->isCompletionPolicyPreview())
    {
        QJsonArray rules = m_configuration[u"enabled"_s].toBool() ? m_configuration[u"rules"_s].toArray() : QJsonArray();
        // Apply the simple download option only to new completion events. An
        // explicit terminal action in the user's rules takes precedence.
        if (notifyFinished && Preferences::instance()->isAutoRemoveCompletedTorrentsEnabled())
        {
            rules.append(QJsonObject {{u"id"_s, u"qbutt-auto-remove-completed"_s}, {u"enabled"_s, true}
                , {u"match"_s, QJsonObject()}, {u"actions"_s, QJsonArray {u"remove_torrent"_s}}});
        }
        for (const QJsonValue &value : evaluate(torrent, {{u"rules"_s, rules}}))
        {
            QJsonObject claim = value.toObject();
            if (claim[u"already_claimed"_s].toBool())
                continue;
            claim.remove(u"already_claimed"_s);
            claim[u"time"_s] = QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs);
            claim[u"status"_s] = u"claimed"_s;
            claims.append(claim);
        }
        QJsonArray entries = m_journal;
        for (const QJsonValue &claim : claims)
            entries.append(claim);
        if (!claims.isEmpty())
        {
#ifdef QBUTT_COMPLETION_FAULTS
            fault("before_claim");
#endif
            m_error = saveJournal(entries);
#ifdef QBUTT_COMPLETION_FAULTS
            if (m_error.isEmpty())
                fault("claimed");
#endif
            success = m_error.isEmpty();
        }
    }
    QString terminal;
    if (success)
    {
        for (const QJsonValue &value : claims)
        {
            const QJsonObject claim = value.toObject();
            m_executed.insert(claim[u"key"_s].toString());
            const QJsonArray actions = claim[u"actions"_s].toArray();
            LogMsg(tr("Completion policy %1 claimed for torrent %2. Key: %3")
                .arg(claim[u"rule"_s].toString(), torrent->name(), claim[u"key"_s].toString()));
            if (actions.contains(u"notify"_s))
                emit notified(tr("Completion policy"), tr("%1: %2").arg(torrent->name(), claim[u"rule"_s].toString()));
            for (const QString &action : {u"stop"_s, u"remove_torrent"_s, u"delete_data"_s})
            {
                if (actions.contains(action))
                    terminal = action;
            }
        }
    }
    torrent->endCompletion(success && terminal.isEmpty());
    m_current.clear();
    if (success && notifyFinished)
    {
        m_notifySession = true;
        emit wantedFilesCommitted(torrent);
    }
    if (torrent && success && terminal.isEmpty())
        m_session->processTorrentShareLimits(torrent, true);
    bool dispatched = true;
    if (success && ((terminal == u"remove_torrent") || (terminal == u"delete_data")))
    {
        dispatched = !torrent || (torrent->isReadyForCompletion()
            && m_session->removeTorrent(torrent->id(), (terminal == u"delete_data")
                ? TorrentRemoveOption::RemoveContent : TorrentRemoveOption::KeepContent));
        if (!dispatched && torrent)
            torrent->setCompletionPolicyPreview(true);
    }
    if (success && !claims.isEmpty())
    {
#ifdef QBUTT_COMPLETION_FAULTS
        fault("dispatched");
#endif
        QJsonArray entries = m_journal;
        for (qsizetype i = entries.size() - claims.size(); i < entries.size(); ++i)
        {
            QJsonObject entry = entries[i].toObject();
            entry[u"status"_s] = dispatched ? u"dispatched"_s : u"blocked"_s;
            entries[i] = entry;
        }
        m_error = saveJournal(entries);
    }
    if (!success && torrent)
        torrent->setCompletionPolicyPreview(true);
    if (!success)
        LogMsg(tr("Completion action held: native disk or resume persistence failed. %1").arg(m_error), Log::WARNING);
    emit changed();
}
