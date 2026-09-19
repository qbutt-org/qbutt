/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "networkfilterwidget.h"

#include <functional>
#include <QComboBox>
#include <QFormLayout>
#include <QJsonArray>
#include <QJsonObject>
#include <QSet>
#include <QSignalBlocker>

#include "base/bittorrent/torrent.h"
#include "base/global.h"
#include "base/net/pathmanager.h"
#include "gui/transferlistwidget.h"

namespace
{
    const QString NO_ACTIVE_PATH = u"no-active-path"_s;

    class PathFilterComboBox final : public QComboBox
    {
    public:
        using QComboBox::QComboBox;

        std::function<void()> beforePopup;

    protected:
        void showPopup() override
        {
            if (beforePopup)
                beforePopup();
            QComboBox::showPopup();
        }
    };
}

NetworkFilterWidget::NetworkFilterWidget(QWidget *parent, TransferListWidget *transferList)
    : QWidget {parent}
    , m_transferList {transferList}
    , m_paths {new PathFilterComboBox {this}}
    , m_sources {new QComboBox {this}}
{
    setObjectName(u"networkFilters"_s);
    m_paths->setObjectName(u"pathFilter"_s);
    m_sources->setObjectName(u"sourceFilter"_s);
    m_paths->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    m_sources->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);

    m_sources->addItem(tr("All peer sources"));
    m_sources->addItem(tr("Tracker"), BitTorrent::TrackerPeerSource);
    m_sources->addItem(tr("DHT"), BitTorrent::DHTPeerSource);
    m_sources->addItem(tr("Peer exchange"), BitTorrent::PeXPeerSource);
    m_sources->addItem(tr("Local discovery"), BitTorrent::LSDPeerSource);
    m_sources->addItem(tr("Resume data"), BitTorrent::ResumeDataPeerSource);
    m_sources->addItem(tr("Incoming"), BitTorrent::IncomingPeerSource);
    m_sources->addItem(tr("Web seed"), BitTorrent::WebSeedPeerSource);
    m_sources->addItem(tr("No active peer source"), 0);

    static_cast<PathFilterComboBox *>(m_paths)->beforePopup = [this]
    {
        m_transferList->getSourceModel()->refreshNetworkCatalog();
    };

    auto *layout = new QFormLayout {this};
    layout->setContentsMargins(4, 0, 4, 2);
    layout->addRow(tr("Path:"), m_paths);
    layout->addRow(tr("Source:"), m_sources);

    refreshPaths();
    connect(m_paths, &QComboBox::currentIndexChanged, this, &NetworkFilterWidget::applyPath);
    connect(m_sources, &QComboBox::currentIndexChanged, this, &NetworkFilterWidget::applySource);
    connect(Net::PathManager::instance(), &Net::PathManager::changed, this, &NetworkFilterWidget::refreshPaths);
    connect(m_transferList->getSourceModel(), &TransferListModel::networkPathsChanged,
        this, &NetworkFilterWidget::refreshPaths);
}

void NetworkFilterWidget::toggleFilter(const bool enabled)
{
    m_filterEnabled = enabled;
    setVisible(enabled);
    if (enabled)
    {
        applyPath();
        applySource();
    }
    else
    {
        m_transferList->applyPathFilter({});
        m_transferList->applySourceFilter({});
    }
}

void NetworkFilterWidget::refreshPaths()
{
    const bool hasSelection = m_paths->currentIndex() > 0;
    const QString selected = m_paths->currentData().toString();
    {
        const QSignalBlocker blocker {m_paths};
        m_paths->clear();
        m_paths->addItem(tr("All paths"));
        m_paths->addItem(tr("Unmanaged/default route"), u"0:0"_s);
        QSet<QString> addedPaths {u"0:0"_s};
        const QJsonArray paths = Net::PathManager::instance()->statusData().value(u"paths"_s).toArray();
        for (const QJsonValue &value : paths)
        {
            const QJsonObject path = value.toObject();
            const QString pathId = path.value(u"pathId"_s).toString();
            const qint64 generation = path.value(u"generation"_s).toInteger(-1);
            if (pathId.isEmpty() || (generation < 0))
                continue;
            const QString pathKey = pathId + u':' + QString::number(generation);
            if (addedPaths.contains(pathKey))
                continue;
            QString label = path.value(u"proxyName"_s).toString();
            if (label.isEmpty())
                label = path.value(u"edgeId"_s).toString();
            m_paths->addItem(label.isEmpty() ? tr("Path %1 · generation %2").arg(pathId).arg(generation)
                                            : tr("%1 · generation %2").arg(label).arg(generation), pathKey);
            addedPaths.insert(pathKey);
        }
        for (const QString &pathKey : m_transferList->getSourceModel()->networkPaths())
        {
            if (addedPaths.contains(pathKey))
                continue;
            const qsizetype separator = pathKey.lastIndexOf(u':');
            if (separator <= 0)
                continue;
            const QString pathId = pathKey.first(separator);
            const QString generation = pathKey.sliced(separator + 1);
            m_paths->addItem(tr("Observed path %1 · generation %2").arg(pathId, generation), pathKey);
            addedPaths.insert(pathKey);
        }
        m_paths->addItem(tr("No active path"), NO_ACTIVE_PATH);
        const int index = hasSelection ? m_paths->findData(selected) : -1;
        m_paths->setCurrentIndex(index >= 0 ? index : 0);
    }
    if (m_filterEnabled)
        applyPath();
}

void NetworkFilterWidget::applyPath()
{
    if (!m_filterEnabled)
        return;
    if (m_paths->currentIndex() == 0)
        m_transferList->applyPathFilter({});
    else
    {
        const QString pathId = m_paths->currentData().toString();
        m_transferList->applyPathFilter((pathId == NO_ACTIVE_PATH) ? QString {} : pathId);
    }
}

void NetworkFilterWidget::applySource()
{
    if (!m_filterEnabled)
        return;
    if (m_sources->currentIndex() == 0)
        m_transferList->applySourceFilter({});
    else
        m_transferList->applySourceFilter(m_sources->currentData().toInt());
}
