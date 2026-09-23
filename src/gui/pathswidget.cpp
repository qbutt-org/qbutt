/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "pathswidget.h"

#ifdef Q_OS_WIN
#include <winsock2.h>
#include <ws2ipdef.h>
#include <iphlpapi.h>
#endif

#include <QBrush>
#include <QCheckBox>
#include <QComboBox>
#include <QFont>
#include <QFormLayout>
#include <QFrame>
#include <QHeaderView>
#include <QHBoxLayout>
#include <QIcon>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QMap>
#include <QNetworkInterface>
#include <QPainter>
#include <QPalette>
#include <QPen>
#include <QPixmap>
#include <QPushButton>
#include <QSignalBlocker>
#include <QSizePolicy>
#include <QSpinBox>
#include <QTreeWidget>
#include <QVBoxLayout>

#include "base/global.h"
#include "base/net/pathmanager.h"
#include "base/utils/misc.h"
#include "uithememanager.h"

namespace
{
    QIcon pinIcon()
    {
        QPixmap image {16, 16};
        image.fill(Qt::transparent);
        QPainter painter {&image};
        painter.setRenderHint(QPainter::Antialiasing);
        QPen pen {QColor {0, 157, 247}};
        pen.setWidthF(1.5);
        painter.setPen(pen);
        painter.setBrush(QColor {0, 157, 247});
        painter.drawRoundedRect(QRectF {5, 1, 6, 3}, 1, 1);
        painter.setBrush(Qt::NoBrush);
        painter.drawLine(QPointF {6, 4}, QPointF {5, 9});
        painter.drawLine(QPointF {10, 4}, QPointF {11, 9});
        painter.drawLine(QPointF {5, 9}, QPointF {11, 9});
        painter.drawLine(QPointF {8, 9}, QPointF {8, 15});
        painter.end();
        return QIcon {image};
    }
}

PathsWidget::PathsWidget(QWidget *parent)
    : QGroupBox(tr("Mihomo subscription"), parent)
    , m_manager {Net::PathManager::instance()}
    , m_url {new QLineEdit(this)}
    , m_nodeFilter {new QLineEdit(this)}
    , m_nodes {new QTreeWidget(this)}
    , m_enabled {new QCheckBox(tr("Use Mihomo"), this)}
    , m_subscriptionFormat {new QComboBox(this)}
    , m_interfaces {new QComboBox(this)}
    , m_dnsServer {new QLineEdit(this)}
    , m_bootstrapServer {new QLineEdit(this)}
    , m_dnsFamily {new QComboBox(this)}
    , m_dnsApply {new QPushButton(tr("Save DNS"), this)}
    , m_gatewayHeading {new QLabel(this)}
    , m_gatewayControlAddress {new QLineEdit(this)}
    , m_gatewayDatagramAddress {new QLineEdit(this)}
    , m_gatewayServerName {new QLineEdit(this)}
    , m_gatewayCaPath {new QLineEdit(this)}
    , m_gatewayCertificatePath {new QLineEdit(this)}
    , m_gatewayPrivateKeyPath {new QLineEdit(this)}
    , m_gatewayPort {new QSpinBox(this)}
    , m_gatewayTcp {new QCheckBox(tr("TCP"), this)}
    , m_gatewayUdp {new QCheckBox(tr("UDP / uTP / DHT"), this)}
    , m_gatewayApply {new QPushButton(tr("Save gateway"), this)}
    , m_status {new QLabel(this)}
{
    auto *layout = new QVBoxLayout(this);
    m_enabled->setObjectName(u"mihomoEnabled"_s);
    layout->addWidget(m_enabled);
    auto *form = new QFormLayout;
    form->setRowWrapPolicy(QFormLayout::WrapLongRows);
    m_url->setObjectName(u"mihomoSubscriptionUrl"_s);
    m_url->setPlaceholderText(u"https://…"_s);
    m_url->setText(m_manager->subscriptionUrl());
    form->addRow(tr("Subscription:"), m_url);
    m_nodes->setObjectName(u"mihomoNodes"_s);
    m_nodes->setColumnCount(3);
    m_nodes->setHeaderLabels({tr("Server / protocol"), tr("Connection"), tr("Relay traffic")});
    m_nodes->header()->resizeSection(0, 230);
    m_nodes->header()->resizeSection(1, 170);
    m_nodes->header()->setStretchLastSection(true);
    m_nodes->setMaximumHeight(260);
    m_nodes->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    m_nodes->setFrameShape(QFrame::NoFrame);
    m_nodes->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_nodeFilter->setObjectName(u"mihomoNodeFilter"_s);
    m_nodeFilter->setPlaceholderText(tr("Find a server or protocol"));
    form->addRow(QString(), m_nodeFilter);
    form->addRow(tr("Servers:"), m_nodes);
    form->setRowVisible(m_nodeFilter, false);
    form->setRowVisible(m_nodes, false);
    m_interfaces->setObjectName(u"mihomoPhysicalInterface"_s);
    m_interfaces->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    m_interfaces->setMinimumContentsLength(12);
    m_interfaces->addItem(tr("Choose a network adapter"), QString());
    const QList<QNetworkInterface> interfaces = QNetworkInterface::allInterfaces();
    for (const QNetworkInterface &iface : interfaces)
    {
        if (!iface.flags().testFlag(QNetworkInterface::IsUp)
            || !iface.flags().testFlag(QNetworkInterface::IsRunning)
            || iface.flags().testFlag(QNetworkInterface::IsLoopBack)
            || ((iface.type() != QNetworkInterface::Ethernet) && (iface.type() != QNetworkInterface::Wifi)))
        {
            continue;
        }
#ifdef Q_OS_WIN
        MIB_IF_ROW2 row {};
        row.InterfaceIndex = static_cast<NET_IFINDEX>(iface.index());
        if ((GetIfEntry2(&row) != NO_ERROR) || !row.InterfaceAndOperStatusFlags.HardwareInterface)
            continue;
        const QString bindName = iface.humanReadableName();
#else
        const QString bindName = iface.name();
#endif
        m_interfaces->addItem(iface.humanReadableName(), bindName);
        m_interfaces->setItemData(m_interfaces->count() - 1, iface.index(), Qt::UserRole + 1);
    }
    const int savedInterface = m_interfaces->findData(m_manager->interfaceName());
    if (savedInterface > 0)
        m_interfaces->setCurrentIndex(savedInterface);
    else if (m_interfaces->count() > 1)
    {
#ifdef Q_OS_WIN
        SOCKADDR_IN destination {};
        destination.sin_family = AF_INET;
        destination.sin_addr.S_un.S_addr = htonl(0x01010101);
        NET_IFINDEX routeInterface = 0;
        if (GetBestInterfaceEx(reinterpret_cast<SOCKADDR *>(&destination), &routeInterface) == NO_ERROR)
        {
            const int routeIndex = m_interfaces->findData(static_cast<int>(routeInterface), Qt::UserRole + 1);
            if (routeIndex > 0)
                m_interfaces->setCurrentIndex(routeIndex);
        }
#endif
        if (m_interfaces->currentIndex() == 0)
            m_interfaces->setCurrentIndex(1);
    }
    form->addRow(tr("Network adapter:"), m_interfaces);
    form->setRowVisible(m_interfaces, m_interfaces->count() != 2);
    layout->addLayout(form);

    m_status->setWordWrap(true);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setObjectName(u"mihomoPathStatus"_s);
    layout->addWidget(m_status);

    auto *advancedHeading = new QLabel(tr("Advanced settings"), this);
    QFont headingFont = advancedHeading->font();
    headingFont.setBold(true);
    advancedHeading->setFont(headingFont);
    layout->addSpacing(12);
    layout->addWidget(advancedHeading);
    auto *dnsForm = new QFormLayout;
    dnsForm->setRowWrapPolicy(QFormLayout::WrapLongRows);
    m_subscriptionFormat->setObjectName(u"mihomoSubscriptionFormat"_s);
    m_subscriptionFormat->addItem(tr("Automatic"), u"auto"_s);
    m_subscriptionFormat->addItem(tr("Mihomo YAML"), u"mihomo"_s);
    m_subscriptionFormat->addItem(tr("Base64 or links"), u"base64"_s);
    m_subscriptionFormat->setCurrentIndex(m_subscriptionFormat->findData(m_manager->subscriptionFormat()));
    dnsForm->addRow(tr("Subscription format:"), m_subscriptionFormat);
    m_dnsServer->setObjectName(u"mihomoDnsServer"_s);
    m_bootstrapServer->setObjectName(u"mihomoBootstrapServer"_s);
    m_dnsFamily->setObjectName(u"mihomoDnsFamily"_s);
    m_dnsApply->setObjectName(u"mihomoSaveDns"_s);
    m_dnsServer->setToolTip(tr("IP:port. Resolve torrent addresses through the connected node."));
    m_bootstrapServer->setToolTip(tr("IP:port. Resolve the node's own address through the network adapter."));
    m_dnsApply->setToolTip(tr("Applies when a node reconnects."));
    m_dnsFamily->addItem(tr("IPv4 and IPv6"), u"dual"_s);
    m_dnsFamily->addItem(tr("IPv4 only"), u"ipv4"_s);
    m_dnsFamily->addItem(tr("IPv6 only"), u"ipv6"_s);
    dnsForm->addRow(tr("DNS server:"), m_dnsServer);
    dnsForm->addRow(tr("Bootstrap DNS:"), m_bootstrapServer);
    dnsForm->addRow(tr("Destination addresses:"), m_dnsFamily);
    dnsForm->addRow(QString(), m_dnsApply);
    layout->addLayout(dnsForm);
    const auto loadDnsSettings = [this]()
    {
        const QJsonObject dns = m_manager->dnsPolicy();
        m_dnsServer->setText(dns.value(u"server"_s).toString());
        m_bootstrapServer->setText(dns.value(u"bootstrapServer"_s).toString());
        m_dnsFamily->setCurrentIndex(m_dnsFamily->findData(dns.value(u"family"_s).toString()));
    };
    loadDnsSettings();
    connect(m_manager, &Net::PathManager::dnsPolicyChanged, this, loadDnsSettings);
    connect(m_dnsApply, &QPushButton::clicked, this, [this]()
    {
        m_manager->setDnsPolicy(m_dnsServer->text(), m_bootstrapServer->text(), m_dnsFamily->currentData().toString());
    });

    m_gatewayHeading->setWordWrap(true);
    layout->addWidget(m_gatewayHeading);
    auto *gatewayForm = new QFormLayout;
    gatewayForm->setRowWrapPolicy(QFormLayout::WrapLongRows);
    m_gatewayControlAddress->setObjectName(u"mihomoGatewayControlAddress"_s);
    m_gatewayDatagramAddress->setObjectName(u"mihomoGatewayDatagramAddress"_s);
    m_gatewayServerName->setObjectName(u"mihomoGatewayServerName"_s);
    m_gatewayCaPath->setObjectName(u"mihomoGatewayCaPath"_s);
    m_gatewayCertificatePath->setObjectName(u"mihomoGatewayCertificatePath"_s);
    m_gatewayPrivateKeyPath->setObjectName(u"mihomoGatewayPrivateKeyPath"_s);
    m_gatewayPort->setObjectName(u"mihomoGatewayPort"_s);
    m_gatewayTcp->setObjectName(u"mihomoGatewayTcp"_s);
    m_gatewayUdp->setObjectName(u"mihomoGatewayUdp"_s);
    m_gatewayApply->setObjectName(u"mihomoSaveGateway"_s);
    m_gatewayControlAddress->setPlaceholderText(u"gateway.example:443"_s);
    m_gatewayDatagramAddress->setPlaceholderText(u"gateway.example:443"_s);
    m_gatewayServerName->setPlaceholderText(u"gateway.example"_s);
    m_gatewayPort->setRange(0, 65535);
    m_gatewayPort->setSpecialValueText(tr("Automatic"));
    auto *protocols = new QHBoxLayout;
    protocols->addWidget(m_gatewayTcp);
    protocols->addWidget(m_gatewayUdp);
    protocols->addStretch();
    gatewayForm->addRow(tr("Control endpoint:"), m_gatewayControlAddress);
    gatewayForm->addRow(tr("Datagram endpoint:"), m_gatewayDatagramAddress);
    gatewayForm->addRow(tr("TLS server name:"), m_gatewayServerName);
    gatewayForm->addRow(tr("CA certificate:"), m_gatewayCaPath);
    gatewayForm->addRow(tr("Client certificate:"), m_gatewayCertificatePath);
    gatewayForm->addRow(tr("Client private key:"), m_gatewayPrivateKeyPath);
    gatewayForm->addRow(tr("Requested port:"), m_gatewayPort);
    gatewayForm->addRow(tr("Listeners:"), protocols);
    gatewayForm->addRow(QString(), m_gatewayApply);
    m_gatewayTcp->setToolTip(tr("Receive connections through your public gateway."));
    m_gatewayUdp->setToolTip(tr("Receive connections through your public gateway."));
    layout->addLayout(gatewayForm);
    loadGatewaySettings();
    connect(m_gatewayUdp, &QCheckBox::toggled, this, &PathsWidget::refreshState);
    connect(m_gatewayApply, &QPushButton::clicked, this, [this]()
    {
        const QString serverId = selectedServerId();
        if (serverId.isEmpty())
            return;
        m_manager->setGatewayConfiguration({
            {u"controlAddress"_s, m_gatewayControlAddress->text()},
            {u"datagramAddress"_s, m_gatewayDatagramAddress->text()},
            {u"serverName"_s, m_gatewayServerName->text()},
            {u"caPath"_s, m_gatewayCaPath->text()},
            {u"certificatePath"_s, m_gatewayCertificatePath->text()},
            {u"privateKeyPath"_s, m_gatewayPrivateKeyPath->text()},
            {u"port"_s, m_gatewayPort->value()},
            {u"tcp"_s, m_gatewayTcp->isChecked()},
            {u"udp"_s, m_gatewayUdp->isChecked()}}, serverId);
    });

    connect(m_url, &QLineEdit::editingFinished, this, [this]()
    {
        if (m_manager->isBusy() || !m_enabled->isChecked() || !m_url->isModified())
            return;
        m_url->setModified(false);
        m_manager->refreshSubscription(m_url->text());
    });
    connect(m_subscriptionFormat, &QComboBox::currentIndexChanged, this, [this]()
    {
        if (!m_manager->setSubscriptionFormat(m_subscriptionFormat->currentData().toString()))
        {
            const QSignalBlocker blocker(m_subscriptionFormat);
            m_subscriptionFormat->setCurrentIndex(m_subscriptionFormat->findData(m_manager->subscriptionFormat()));
        }
    });
    connect(m_nodeFilter, &QLineEdit::textChanged, this, &PathsWidget::filterNodes);
    connect(m_nodes, &QTreeWidget::currentItemChanged, this,
        [this](QTreeWidgetItem *current, QTreeWidgetItem *previous)
    {
        const QTreeWidgetItem *oldServer = previous && previous->parent() ? previous->parent() : previous;
        const QTreeWidgetItem *newServer = current && current->parent() ? current->parent() : current;
        if (oldServer != newServer)
            loadGatewaySettings();
        else
            refreshState();
    });
    connect(m_nodes, &QTreeWidget::itemChanged, this, [this](QTreeWidgetItem *item)
    {
        if (!item->parent())
            saveSelection();
    });
    connect(m_nodes, &QTreeWidget::itemClicked, this, [this](QTreeWidgetItem *item, const int column)
    {
        if (!item->parent() || (column != 0))
            return;
        const QString edge = item->parent()->data(0, Qt::UserRole).toString();
        const QString name = item->data(0, Qt::UserRole).toString();
        const QString preferred = m_manager->preferredTransport(edge);
        m_manager->setPreferredTransport(edge, preferred == name ? QString() : name);
        refreshState();
    });
    connect(m_enabled, &QCheckBox::toggled, this, [this](const bool enabled)
    {
        m_setupIntent = enabled;
        if (enabled)
            activateSelection();
        else if (m_manager->managedEnabled() || m_manager->isBusy())
            m_manager->setManagedEnabled(false);
        refreshState();
    });
    connect(m_interfaces, &QComboBox::currentIndexChanged, this, [this]()
    {
        const bool managed = m_manager->managedEnabled();
        if (managed && !m_manager->setManagedEnabled(false))
        {
            refreshState();
            return;
        }
        if (managed)
            m_setupIntent = true;
        activateSelection();
        refreshState();
    });
    connect(m_manager, &Net::PathManager::changed, this, &PathsWidget::refreshState);
    connect(UIThemeManager::instance(), &UIThemeManager::themeChanged, this, &PathsWidget::refreshState);
    connect(m_manager, &Net::PathManager::proxiesLoaded, this, [this, form](const QJsonArray &proxies)
    {
        const QStringList selected = m_manager->selectedNodes();
        const QString previousServer = selectedServerId();
        QMap<QString, QList<QJsonObject>> groups;
        const QSignalBlocker nodesBlocker(m_nodes);
        m_nodes->clear();
        for (const QJsonValue &value : proxies)
        {
            const QJsonObject node = value.toObject();
            const QString name = node.value(u"name"_s).toString();
            const QString edge = node.value(u"edgeId"_s).toString();
            const QString host = node.value(u"serverHost"_s).toString();
            if (name.isEmpty() || edge.isEmpty() || host.isEmpty())
                continue;
            groups[edge].append(node);
        }
        QStringList retained;
        for (auto it = groups.cbegin(); it != groups.cend(); ++it)
        {
            const QList<QJsonObject> &variants = it.value();
            auto *server = new QTreeWidgetItem(m_nodes, {variants.first().value(u"serverHost"_s).toString()});
            server->setToolTip(0, server->text(0));
            server->setData(0, Qt::UserRole, it.key());
            server->setFlags(server->flags() | Qt::ItemIsUserCheckable);
            QStringList names;
            bool checked = false;
            for (const QJsonObject &node : variants)
            {
                const QString name = node.value(u"name"_s).toString();
                names.append(name);
                checked |= selected.contains(name);
                if (variants.size() > 1)
                {
                    auto *item = new QTreeWidgetItem(server, {u"%1 (%2)"_s.arg(name, node.value(u"type"_s).toString())});
                    item->setData(0, Qt::UserRole, name);
                    item->setFlags(item->flags() & ~Qt::ItemIsUserCheckable);
                }
            }
            server->setData(0, Qt::UserRole + 1, names);
            server->setCheckState(0, checked ? Qt::Checked : Qt::Unchecked);
            if (checked)
                retained.append(names);
        }
        m_nodes->sortItems(0, Qt::AscendingOrder);
        m_nodes->expandAll();
        QTreeWidgetItem *currentServer = nullptr;
        for (int row = 0; row < m_nodes->topLevelItemCount(); ++row)
        {
            QTreeWidgetItem *server = m_nodes->topLevelItem(row);
            if (server->data(0, Qt::UserRole) == previousServer)
                currentServer = server;
        }
        if (!currentServer && (m_nodes->topLevelItemCount() > 0))
            currentServer = m_nodes->topLevelItem(0);
        m_nodes->setCurrentItem(currentServer);
        loadGatewaySettings();
        filterNodes();
        form->setRowVisible(m_nodeFilter, m_nodes->topLevelItemCount() > 0);
        form->setRowVisible(m_nodes, m_nodes->topLevelItemCount() > 0);
        QStringList oldNames = selected;
        QStringList newNames = retained;
        oldNames.sort();
        newNames.sort();
        if (newNames != oldNames)
            m_manager->setSelectedNodes(retained);
        activateSelection();
        refreshState();
    });
    refreshState();
    if (!m_manager->configurationPath().isEmpty() && !m_manager->isBusy())
        m_manager->inspectConfiguration(m_manager->configurationPath());
}

void PathsWidget::activateSelection()
{
    if (m_setupIntent && !m_manager->managedEnabled() && !m_manager->isBusy()
        && !m_manager->selectedNodes().isEmpty() && !m_interfaces->currentData().toString().isEmpty())
    {
        m_manager->setManagedEnabled(true, m_interfaces->currentData().toString());
    }
}

void PathsWidget::saveSelection()
{
    QStringList selected;
    for (int row = 0; row < m_nodes->topLevelItemCount(); ++row)
    {
        const QTreeWidgetItem *server = m_nodes->topLevelItem(row);
        if (server->checkState(0) == Qt::Checked)
            selected.append(server->data(0, Qt::UserRole + 1).toStringList());
    }
    if (m_manager->setSelectedNodes(selected))
        activateSelection();
    refreshState();
}

void PathsWidget::refreshState()
{
    const bool busy = m_manager->isBusy();
    const bool managed = m_manager->managedEnabled();
    if (managed)
        m_setupIntent = false;
    const bool enabled = managed || m_setupIntent;
    const QJsonObject state = m_manager->statusData();
    QMap<QString, QJsonObject> active;
    for (const QJsonValue &value : state.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        if (path.value(u"edgeId"_s) != u"native"_s)
        {
            const QString edge = path.value(u"edgeId"_s).toString();
            const bool reachable = path.value(u"health"_s).toObject().value(u"state"_s) == u"reachable"_s;
            if (!active.contains(edge) || reachable)
                active.insert(edge, path);
        }
    }
    QStringList pending;
    QStringList failed;
    for (const QJsonValue &value : state.value(u"pendingNodes"_s).toArray())
        pending.append(value.toString());
    for (const QJsonValue &value : state.value(u"failedNodes"_s).toArray())
        failed.append(value.toString());
    const QStringList selected = m_manager->selectedNodes();
    const QSignalBlocker nodesBlocker(m_nodes);
    for (int row = 0; row < m_nodes->topLevelItemCount(); ++row)
    {
        QTreeWidgetItem *server = m_nodes->topLevelItem(row);
        const QJsonObject path = active.value(server->data(0, Qt::UserRole).toString());
        const QString health = path.value(u"health"_s).toObject().value(u"state"_s).toString();
        const bool pathOpen = path.value(u"open"_s).toBool();
        const bool reachable = pathOpen && (health == u"reachable"_s);
        const bool dnsCheckFailed = pathOpen && (health == u"failed"_s);
        const QStringList names = server->data(0, Qt::UserRole + 1).toStringList();
        bool checked = false;
        bool waiting = false;
        bool unavailable = false;
        for (const QString &name : names)
        {
            checked |= selected.contains(name);
            waiting |= pending.contains(name);
            unavailable |= failed.contains(name);
        }
        server->setCheckState(0, checked ? Qt::Checked : Qt::Unchecked);
        waiting |= health == u"checking"_s;
        const bool disconnecting = !checked && pathOpen;
        const bool failedServer = checked && managed && !pathOpen && !waiting
            && (unavailable || health == u"failed"_s);
        server->setText(1, disconnecting ? tr("Disconnecting") : !checked || !managed ? QString()
            : reachable ? tr("Reachable") : dnsCheckFailed ? tr("DNS check failed") : waiting ? tr("Connecting")
            : failedServer ? tr("Cannot connect") : tr("Unknown"));
        server->setToolTip(1, dnsCheckFailed
            ? tr("The TCP DNS check through this node failed; existing peer traffic may still work.") : QString());
        const QBrush color = palette().brush(failedServer ? QPalette::Disabled : QPalette::Active, QPalette::Text);
        for (int column = 0; column < m_nodes->columnCount(); ++column)
            server->setForeground(column, color);
        const QString preferred = m_manager->preferredTransport(server->data(0, Qt::UserRole).toString());
        static const QIcon pinnedIcon = pinIcon();
        for (int child = 0; child < server->childCount(); ++child)
        {
            QTreeWidgetItem *node = server->child(child);
            const QString name = node->data(0, Qt::UserRole).toString();
            node->setIcon(0, preferred == name ? pinnedIcon : QIcon {});
            node->setToolTip(0, preferred == name
                ? tr("Preferred protocol. Click to return to automatic selection.")
                : tr("Click to prefer this protocol. Click again for automatic selection."));
            QString connection;
            if (managed && (checked || disconnecting))
            {
                if (pathOpen && (path.value(u"proxyName"_s) == name))
                    connection = health == u"reachable"_s ? tr("Active · reachable")
                        : health == u"checking"_s ? tr("Checking")
                        : health == u"failed"_s ? tr("DNS check failed") : tr("Unknown");
                else if (failed.contains(name))
                    connection = tr("Attempt failed");
                else if (pending.contains(name))
                    connection = tr("Connecting");
            }
            node->setText(1, connection);
            node->setToolTip(1, pathOpen && (path.value(u"proxyName"_s) == name) && dnsCheckFailed
                ? tr("The TCP DNS check through this node failed; existing peer traffic may still work.") : QString());
        }
        const QJsonObject wire = path.value(u"wire"_s).toObject();
        if (!wire.isEmpty())
        {
            const QJsonObject rate = path.value(u"relayRate"_s).toObject();
            server->setText(2, tr("↓ %1 (%2)  ↑ %3 (%4)").arg(
                Utils::Misc::friendlyUnit(wire.value(u"relayDownloadBytes"_s).toInteger()),
                Utils::Misc::friendlyUnit(static_cast<qint64>(rate.value(u"downloadBytesPerSecond"_s).toDouble()), true),
                Utils::Misc::friendlyUnit(wire.value(u"relayUploadBytes"_s).toInteger()),
                Utils::Misc::friendlyUnit(static_cast<qint64>(rate.value(u"uploadBytesPerSecond"_s).toDouble()), true)));
            server->setToolTip(2, tr("SOCKS relay bytes for this connection, including protocol overhead; not verified torrent data."));
        }
        else
        {
            server->setText(2, {});
            server->setToolTip(2, {});
        }
    }
    m_url->setEnabled(enabled && !busy);
    m_nodeFilter->setEnabled(enabled && (m_nodes->topLevelItemCount() > 0));
    {
        const QSignalBlocker enabledBlocker(m_enabled);
        m_enabled->setChecked(enabled);
    }
    m_nodes->setEnabled(enabled);
    m_subscriptionFormat->setEnabled(enabled && !busy);
    m_interfaces->setEnabled(enabled && !busy);
    m_dnsServer->setEnabled(enabled && !busy);
    m_bootstrapServer->setEnabled(enabled && !busy);
    m_dnsFamily->setEnabled(enabled && !busy);
    m_dnsApply->setEnabled(enabled && !busy);
    const bool gatewayEditable = enabled && !busy && !selectedServerId().isEmpty();
    m_gatewayControlAddress->setEnabled(gatewayEditable);
    m_gatewayDatagramAddress->setEnabled(gatewayEditable && m_gatewayUdp->isChecked());
    m_gatewayServerName->setEnabled(gatewayEditable);
    m_gatewayCaPath->setEnabled(gatewayEditable);
    m_gatewayCertificatePath->setEnabled(gatewayEditable);
    m_gatewayPrivateKeyPath->setEnabled(gatewayEditable);
    m_gatewayPort->setEnabled(gatewayEditable);
    m_gatewayTcp->setEnabled(gatewayEditable);
    m_gatewayUdp->setEnabled(gatewayEditable);
    m_gatewayApply->setEnabled(gatewayEditable);
    if (enabled && !m_manager->status().isEmpty())
        m_status->setText(m_manager->status());
    else if (m_setupIntent && m_manager->selectedNodes().isEmpty())
        m_status->setText(tr("Choose a node to connect."));
    else if (m_setupIntent && m_interfaces->currentData().toString().isEmpty())
        m_status->setText(tr("Choose a network adapter to connect."));
    else
        m_status->clear();
    m_status->setVisible(!m_status->text().isEmpty());
}

void PathsWidget::filterNodes()
{
    const QString query = m_nodeFilter->text();
    for (int row = 0; row < m_nodes->topLevelItemCount(); ++row)
    {
        QTreeWidgetItem *server = m_nodes->topLevelItem(row);
        const bool serverMatches = server->text(0).contains(query, Qt::CaseInsensitive);
        bool childMatches = serverMatches;
        const bool singleVariant = server->childCount() == 0;
        if (singleVariant)
            childMatches |= server->data(0, Qt::UserRole + 1).toStringList().join(u' ').contains(query, Qt::CaseInsensitive);
        for (int child = 0; child < server->childCount(); ++child)
        {
            QTreeWidgetItem *node = server->child(child);
            const bool matches = serverMatches || node->text(0).contains(query, Qt::CaseInsensitive);
            node->setHidden(!matches);
            childMatches |= matches;
        }
        server->setHidden(!childMatches);
        if (childMatches && !query.isEmpty())
            server->setExpanded(true);
    }
}

QString PathsWidget::selectedServerId() const
{
    const QTreeWidgetItem *item = m_nodes->currentItem();
    if (!item)
        return {};
    const QTreeWidgetItem *server = item->parent() ? item->parent() : item;
    return server->data(0, Qt::UserRole).toString();
}

void PathsWidget::loadGatewaySettings()
{
    const QString serverId = selectedServerId();
    const QTreeWidgetItem *item = m_nodes->currentItem();
    const QTreeWidgetItem *server = item && item->parent() ? item->parent() : item;
    m_gatewayHeading->setText(serverId.isEmpty() ? tr("Incoming connections")
        : tr("Incoming connections for %1").arg(server->text(0)));
    const QJsonObject gateway = serverId.isEmpty() ? QJsonObject {} : m_manager->gatewayConfiguration(serverId);
    m_gatewayControlAddress->setText(gateway.value(u"controlAddress"_s).toString());
    m_gatewayDatagramAddress->setText(gateway.value(u"datagramAddress"_s).toString());
    m_gatewayServerName->setText(gateway.value(u"serverName"_s).toString());
    m_gatewayCaPath->setText(gateway.value(u"caPath"_s).toString());
    m_gatewayCertificatePath->setText(gateway.value(u"certificatePath"_s).toString());
    m_gatewayPrivateKeyPath->setText(gateway.value(u"privateKeyPath"_s).toString());
    m_gatewayPort->setValue(gateway.value(u"port"_s).toInt());
    const QSignalBlocker udpBlocker(m_gatewayUdp);
    m_gatewayTcp->setChecked(gateway.value(u"tcp"_s).toBool());
    m_gatewayUdp->setChecked(gateway.value(u"udp"_s).toBool());
    refreshState();
}
