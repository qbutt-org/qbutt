// SPDX-License-Identifier: GPL-2.0-or-later
#include "variantfiltermodel.h"

#include <algorithm>

#include <QCollator>
#include <QMap>
#include <QVariantMap>

namespace
{
    using Options = QMap<QString, QVariantMap>;

    void insertOption(Options &options, const QString &value)
    {
        if (!value.isEmpty())
            options.insert(value, {{"value", value}, {"label", value}});
    }

    QVariantList sortedOptions(const Options &options, const QCollator &collator)
    {
        QVariantList result;
        result.reserve(options.size());
        for (const QVariantMap &option : options)
            result.append(option);

        std::stable_sort(result.begin(), result.end(), [&collator](const QVariant &left, const QVariant &right)
        {
            return collator.compare(left.toMap().value("label").toString(),
                right.toMap().value("label").toString()) < 0;
        });
        return result;
    }
}

Content::VariantFilterModel::VariantFilterModel(QObject *parent)
    : QSortFilterProxyModel {parent}
    , m_optionsTimer {this}
{
    connect(this, &VariantFilterModel::filtersChanged, this, [this]
    {
        m_queryTokens = m_query.simplified().split(' ', Qt::SkipEmptyParts);
        invalidateRowsFilter();
    });
    // ListView must consume model changes before a count binding updates its currentIndex.
    connect(this, &QAbstractItemModel::rowsInserted, this, &VariantFilterModel::countChanged, Qt::QueuedConnection);
    connect(this, &QAbstractItemModel::rowsRemoved, this, &VariantFilterModel::countChanged, Qt::QueuedConnection);
    connect(this, &QAbstractItemModel::modelReset, this, &VariantFilterModel::countChanged, Qt::QueuedConnection);
    connect(this, &QAbstractItemModel::layoutChanged, this, &VariantFilterModel::countChanged, Qt::QueuedConnection);

    m_optionsTimer.setSingleShot(true);
    m_optionsTimer.setInterval(0);
    connect(&m_optionsTimer, &QTimer::timeout, this, &VariantFilterModel::rebuildOptions);
}

void Content::VariantFilterModel::setSourceModel(QAbstractItemModel *source)
{
    if (source == sourceModel())
        return;

    for (const QMetaObject::Connection &connection : m_sourceConnections)
        disconnect(connection);
    m_sourceConnections.clear();

    const QHash<int, QByteArray> roles = source ? source->roleNames() : QHash<int, QByteArray> {};
    m_idRole = roles.key("variantId", -1);
    m_labelRole = roles.key("label", -1);
    m_episodeRole = roles.key("episode", -1);
    m_translationRole = roles.key("translation", -1);
    m_sourceIdRole = roles.key("sourceId", -1);
    m_sourceRole = roles.key("source", -1);
    m_playerRole = roles.key("player", -1);
    m_languagesRole = roles.key("languages", -1);
    m_kindRole = roles.key("kind", -1);
    m_qualityRole = roles.key("quality", -1);
    QSortFilterProxyModel::setSourceModel(source);

    if (source)
    {
        m_sourceConnections = {
            connect(source, &QAbstractItemModel::rowsInserted, this, &VariantFilterModel::scheduleOptionsUpdate),
            connect(source, &QAbstractItemModel::rowsRemoved, this, &VariantFilterModel::scheduleOptionsUpdate),
            connect(source, &QAbstractItemModel::dataChanged, this, &VariantFilterModel::scheduleOptionsUpdate),
            connect(source, &QAbstractItemModel::modelReset, this, &VariantFilterModel::scheduleOptionsUpdate),
            connect(source, &QObject::destroyed, this, &VariantFilterModel::scheduleOptionsUpdate)
        };
    }

    m_optionsTimer.stop();
    rebuildOptions();
}

QVariantList Content::VariantFilterModel::episodeOptions() const
{
    return m_episodeOptions;
}

QVariantList Content::VariantFilterModel::translationOptions() const
{
    return m_translationOptions;
}

QVariantList Content::VariantFilterModel::providerOptions() const
{
    return m_providerOptions;
}

QVariantList Content::VariantFilterModel::languageOptions() const
{
    return m_languageOptions;
}

bool Content::VariantFilterModel::hasTorrents() const
{
    return m_hasTorrents;
}

int Content::VariantFilterModel::count() const
{
    return rowCount();
}

QString Content::VariantFilterModel::idAt(const int row) const
{
    return data(index(row, 0), m_idRole).toString();
}

int Content::VariantFilterModel::indexOfId(const QString &id) const
{
    for (int row = 0; row < rowCount(); ++row)
    {
        if (idAt(row) == id)
            return row;
    }
    return -1;
}

bool Content::VariantFilterModel::filterAcceptsRow(const int sourceRow, const QModelIndex &sourceParent) const
{
    const QModelIndex row = sourceModel()->index(sourceRow, 0, sourceParent);
    const QString episode = row.data(m_episodeRole).toString();
    const QString translation = row.data(m_translationRole).toString();
    const QString sourceId = row.data(m_sourceIdRole).toString();
    const QString player = row.data(m_playerRole).toString();
    const QStringList languages = row.data(m_languagesRole).toStringList();

    // An empty player leaves the selected source's own and external players available.
    if ((!m_episodeFilter.isEmpty() && episode != m_episodeFilter)
        || (!m_translationFilter.isEmpty() && translation != m_translationFilter)
        || (!m_sourceFilter.isEmpty() && sourceId != m_sourceFilter)
        || (!m_playerFilter.isEmpty() && player != m_playerFilter)
        || (!m_languageFilter.isEmpty() && !languages.contains(m_languageFilter))
        || (!m_transportFilter.isEmpty() && row.data(m_kindRole).toString() != m_transportFilter))
    {
        return false;
    }

    if (m_queryTokens.isEmpty())
        return true;

    const QString searchable = QStringList {
        row.data(m_labelRole).toString(),
        episode,
        translation,
        sourceId,
        row.data(m_sourceRole).toString(),
        player,
        languages.join(' '),
        row.data(m_qualityRole).toString()
    }.join(' ');

    return std::all_of(m_queryTokens.cbegin(), m_queryTokens.cend(), [&searchable](const QString &token)
    {
        return searchable.contains(token, Qt::CaseInsensitive);
    });
}

void Content::VariantFilterModel::scheduleOptionsUpdate()
{
    if (!m_optionsTimer.isActive())
        m_optionsTimer.start();
}

void Content::VariantFilterModel::rebuildOptions()
{
    Options episodes;
    Options translations;
    Options providers;
    Options languages;
    bool hasTorrents = false;
    const QAbstractItemModel *source = sourceModel();

    for (int rowIndex = 0; source && rowIndex < source->rowCount(); ++rowIndex)
    {
        const QModelIndex row = source->index(rowIndex, 0);
        insertOption(episodes, row.data(m_episodeRole).toString());
        insertOption(translations, row.data(m_translationRole).toString());
        for (const QString &language : row.data(m_languagesRole).toStringList())
            insertOption(languages, language);

        const QString sourceId = row.data(m_sourceIdRole).toString();
        const QString sourceLabel = row.data(m_sourceRole).toString();
        const QString player = row.data(m_playerRole).toString();
        if (!sourceId.isEmpty() && !sourceLabel.isEmpty())
        {
            const QString value = QString::number(sourceId.size()) + ':' + sourceId + player;
            const QString label = player.isEmpty() ? sourceLabel : sourceLabel + " [" + player + ']';
            providers.insert(value, {
                {"value", value}, {"label", label}, {"source", sourceId}, {"player", player}
            });
        }

        hasTorrents = hasTorrents || row.data(m_kindRole).toString() == "torrent";
    }

    QCollator collator;
    collator.setNumericMode(true);
    collator.setCaseSensitivity(Qt::CaseInsensitive);
    const QVariantList episodeOptions = sortedOptions(episodes, collator);
    const QVariantList translationOptions = sortedOptions(translations, collator);
    const QVariantList providerOptions = sortedOptions(providers, collator);
    const QVariantList languageOptions = sortedOptions(languages, collator);

    if (m_episodeOptions == episodeOptions && m_translationOptions == translationOptions
        && m_providerOptions == providerOptions && m_languageOptions == languageOptions
        && m_hasTorrents == hasTorrents)
    {
        return;
    }

    m_episodeOptions = episodeOptions;
    m_translationOptions = translationOptions;
    m_providerOptions = providerOptions;
    m_languageOptions = languageOptions;
    m_hasTorrents = hasTorrents;
    emit optionsChanged();
}
