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

#include <QCheckBox>
#include <QComboBox>
#include <QFont>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QNetworkInterface>
#include <QPushButton>
#include <QSignalBlocker>
#include <QSpinBox>
#include <QVBoxLayout>

#include "base/global.h"
#include "base/net/pathmanager.h"

PathsWidget::PathsWidget(QWidget *parent)
    : QGroupBox(tr("Mihomo subscription"), parent)
    , m_manager {Net::PathManager::instance()}
    , m_transportForm {new QFormLayout}
    , m_url {new QLineEdit(this)}
    , m_nodeFilter {new QLineEdit(this)}
    , m_nodes {new QListWidget(this)}
    , m_enabled {new QCheckBox(tr("Use Mihomo"), this)}
    , m_sameServer {new QComboBox(this)}
    , m_groupServers {new QPushButton(tr("Group servers"), this)}
    , m_resetServerGroups {new QPushButton(tr("Reset grouping"), this)}
    , m_reserves {new QListWidget(this)}
    , m_interfaces {new QComboBox(this)}
    , m_dnsServer {new QLineEdit(this)}
    , m_bootstrapServer {new QLineEdit(this)}
    , m_dnsFamily {new QComboBox(this)}
    , m_dnsApply {new QPushButton(tr("Save DNS"), this)}
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
    , m_paths {new QListWidget(this)}
    , m_refresh {new QPushButton(tr("Refresh"), this)}
    , m_switch {new QPushButton(tr("Use selected backup"), this)}
    , m_status {new QLabel(this)}
{
    auto *layout = new QVBoxLayout(this);
    auto *form = new QFormLayout;
    auto *subscription = new QHBoxLayout;
    m_url->setObjectName(u"mihomoSubscriptionUrl"_s);
    m_url->setPlaceholderText(u"https://…"_s);
    m_url->setText(m_manager->subscriptionUrl());
    subscription->addWidget(m_url, 1);
    subscription->addWidget(m_refresh);
    form->addRow(tr("Subscription:"), subscription);
    m_nodes->setObjectName(u"mihomoNodes"_s);
    m_nodes->setMaximumHeight(150);
    m_nodes->setToolTip(tr("Check the nodes to use alongside the direct connection."));
    m_nodeFilter->setObjectName(u"mihomoNodeFilter"_s);
    m_nodeFilter->setPlaceholderText(tr("Find a node"));
    form->addRow(QString(), m_nodeFilter);
    form->addRow(tr("Nodes:"), m_nodes);
    m_enabled->setObjectName(u"mihomoEnabled"_s);
    form->addRow(QString(), m_enabled);
    form->setRowVisible(m_nodeFilter, false);
    form->setRowVisible(m_nodes, false);
    m_interfaces->setObjectName(u"mihomoPhysicalInterface"_s);
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

    m_paths->setObjectName(u"mihomoPaths"_s);
    m_paths->setMaximumHeight(110);
    layout->addWidget(m_paths);

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
    auto *serverGrouping = new QHBoxLayout;
    m_sameServer->setObjectName(u"mihomoSameServer"_s);
    m_groupServers->setObjectName(u"mihomoGroupServers"_s);
    m_resetServerGroups->setObjectName(u"mihomoResetServerGroups"_s);
    m_groupServers->setToolTip(tr("Group only nodes you know share one server. Disconnect them first."));
    m_resetServerGroups->setToolTip(tr("Remove manual server groups."));
    serverGrouping->addWidget(m_sameServer, 1);
    serverGrouping->addWidget(m_groupServers);
    serverGrouping->addWidget(m_resetServerGroups);
    m_transportForm->addRow(tr("Same server as:"), serverGrouping);
    m_reserves->setObjectName(u"mihomoReserveTransports"_s);
    m_reserves->setMaximumHeight(75);
    m_reserves->setToolTip(tr("Choose up to three backup connections to the same server."));
    m_transportForm->addRow(tr("Backup connections:"), m_reserves);
    m_switch->setObjectName(u"mihomoSwitchTransport"_s);
    m_transportForm->addRow(QString(), m_switch);
    layout->addLayout(m_transportForm);

    auto *dnsForm = new QFormLayout;
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

    layout->addWidget(new QLabel(tr("Incoming connections"), this));
    auto *gatewayForm = new QFormLayout;
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
    const QJsonObject gateway = m_manager->gatewayConfiguration();
    m_gatewayControlAddress->setText(gateway.value(u"controlAddress"_s).toString());
    m_gatewayDatagramAddress->setText(gateway.value(u"datagramAddress"_s).toString());
    m_gatewayServerName->setText(gateway.value(u"serverName"_s).toString());
    m_gatewayCaPath->setText(gateway.value(u"caPath"_s).toString());
    m_gatewayCertificatePath->setText(gateway.value(u"certificatePath"_s).toString());
    m_gatewayPrivateKeyPath->setText(gateway.value(u"privateKeyPath"_s).toString());
    m_gatewayPort->setValue(gateway.value(u"port"_s).toInt());
    m_gatewayTcp->setChecked(gateway.value(u"tcp"_s).toBool());
    m_gatewayUdp->setChecked(gateway.value(u"udp"_s).toBool());
    connect(m_gatewayUdp, &QCheckBox::toggled, m_gatewayDatagramAddress, &QWidget::setEnabled);
    m_gatewayDatagramAddress->setEnabled(m_gatewayUdp->isChecked());
    connect(m_gatewayApply, &QPushButton::clicked, this, [this]()
    {
        m_manager->setGatewayConfiguration({
            {u"controlAddress"_s, m_gatewayControlAddress->text()},
            {u"datagramAddress"_s, m_gatewayDatagramAddress->text()},
            {u"serverName"_s, m_gatewayServerName->text()},
            {u"caPath"_s, m_gatewayCaPath->text()},
            {u"certificatePath"_s, m_gatewayCertificatePath->text()},
            {u"privateKeyPath"_s, m_gatewayPrivateKeyPath->text()},
            {u"port"_s, m_gatewayPort->value()},
            {u"tcp"_s, m_gatewayTcp->isChecked()},
            {u"udp"_s, m_gatewayUdp->isChecked()}});
    });

    connect(m_refresh, &QPushButton::clicked, this, [this]()
    {
        m_manager->refreshSubscription(m_url->text());
    });
    connect(m_nodeFilter, &QLineEdit::textChanged, this, [this](const QString &query)
    {
        for (int row = 0; row < m_nodes->count(); ++row)
            m_nodes->item(row)->setHidden(!m_nodes->item(row)->text().contains(query, Qt::CaseInsensitive));
    });
    connect(m_nodes, &QListWidget::itemChanged, this, [this](QListWidgetItem *item)
    {
        QStringList selected;
        for (int row = 0; row < m_nodes->count(); ++row)
        {
            const QListWidgetItem *node = m_nodes->item(row);
            if (node->checkState() == Qt::Checked)
                selected.append(node->data(Qt::UserRole).toString());
        }
        if (!m_manager->setSelectedNodes(selected))
        {
            const QSignalBlocker nodesBlocker(m_nodes);
            const QStringList saved = m_manager->selectedNodes();
            for (int row = 0; row < m_nodes->count(); ++row)
            {
                QListWidgetItem *node = m_nodes->item(row);
                node->setCheckState(saved.contains(node->data(Qt::UserRole).toString())
                    ? Qt::Checked : Qt::Unchecked);
            }
        }
        m_nodes->setCurrentItem(item);
    });
    connect(m_enabled, &QCheckBox::toggled, this, [this](const bool enabled)
    {
        if (!m_manager->setManagedEnabled(enabled, m_interfaces->currentData().toString()))
        {
            const QSignalBlocker blocker(m_enabled);
            m_enabled->setChecked(m_manager->managedEnabled());
        }
        refreshState();
    });
    connect(m_groupServers, &QPushButton::clicked, this, [this]()
    {
        m_manager->groupServers(selectedNode(), m_sameServer->currentData().toString());
    });
    connect(m_resetServerGroups, &QPushButton::clicked, m_manager, &Net::PathManager::resetServerGroups);
    connect(m_sameServer, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_switch, &QPushButton::clicked, this, [this]()
    {
        if (m_paths->currentItem() && m_reserves->currentItem())
            m_manager->switchTransport(m_paths->currentItem()->data(Qt::UserRole).toString(),
                m_reserves->currentItem()->data(Qt::UserRole).toString());
    });
    connect(m_nodes, &QListWidget::currentRowChanged, this, &PathsWidget::refreshState);
    connect(m_nodes, &QListWidget::currentRowChanged, this, &PathsWidget::refreshReserves);
    connect(m_interfaces, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_paths, &QListWidget::currentRowChanged, this, &PathsWidget::refreshState);
    connect(m_reserves, &QListWidget::currentRowChanged, this, &PathsWidget::refreshState);
    connect(m_reserves, &QListWidget::itemChanged, this, [this]()
    {
        QStringList selected;
        for (int row = 0; row < m_reserves->count(); ++row)
        {
            const QListWidgetItem *item = m_reserves->item(row);
            if (item->checkState() == Qt::Checked)
                selected.append(item->data(Qt::UserRole).toString());
        }
        if (!m_manager->setReserveNames(selectedNode(), selected))
        {
            const QSignalBlocker blocker(m_reserves);
            const QStringList saved = m_manager->reserveNames(selectedNode());
            for (int row = 0; row < m_reserves->count(); ++row)
            {
                QListWidgetItem *item = m_reserves->item(row);
                item->setCheckState(saved.contains(item->data(Qt::UserRole).toString())
                    ? Qt::Checked : Qt::Unchecked);
            }
        }
        refreshState();
    });
    connect(m_manager, &Net::PathManager::changed, this, &PathsWidget::refreshState);
    connect(m_manager, &Net::PathManager::proxiesLoaded, this, [this, form](const QJsonArray &proxies)
    {
        const QString previous = selectedNode().isEmpty() ? m_manager->proxyName() : selectedNode();
        const QStringList selected = m_manager->selectedNodes();
        const QSignalBlocker nodesBlocker(m_nodes);
        m_nodes->clear();
        for (const QJsonValue &value : proxies)
        {
            const QJsonObject node = value.toObject();
            const QString name = node.value(u"name"_s).toString();
            if (!name.isEmpty())
            {
                auto *item = new QListWidgetItem(u"%1 (%2)"_s.arg(name, node.value(u"type"_s).toString()), m_nodes);
                item->setData(Qt::UserRole, name);
                item->setData(Qt::UserRole + 1, node.value(u"configuredServerId"_s).toString());
                item->setFlags(item->flags() | Qt::ItemIsUserCheckable);
                item->setCheckState(selected.contains(name) ? Qt::Checked : Qt::Unchecked);
            }
        }
        for (int row = 0; row < m_nodes->count(); ++row)
        {
            if (m_nodes->item(row)->data(Qt::UserRole) == previous)
            {
                m_nodes->setCurrentRow(row);
                break;
            }
        }
        if (!m_nodes->currentItem() && (m_nodes->count() > 0))
            m_nodes->setCurrentRow(0);
        for (int row = 0; row < m_nodes->count(); ++row)
            m_nodes->item(row)->setHidden(!m_nodes->item(row)->text().contains(
                m_nodeFilter->text(), Qt::CaseInsensitive));
        form->setRowVisible(m_nodeFilter, m_nodes->count() > 0);
        form->setRowVisible(m_nodes, m_nodes->count() > 0);
        refreshReserves();
        refreshState();
    });
    refreshState();
    if (!m_manager->configurationPath().isEmpty() && !m_manager->isBusy())
        m_manager->inspectConfiguration(m_manager->configurationPath());
}

QString PathsWidget::selectedNode() const
{
    return m_nodes->currentItem() ? m_nodes->currentItem()->data(Qt::UserRole).toString() : QString();
}

void PathsWidget::refreshState()
{
    const bool busy = m_manager->isBusy();
    const QJsonObject state = m_manager->statusData();
    bool managedOpen = false;
    for (const QJsonValue &value : state.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        managedOpen = managedOpen || ((path.value(u"edgeId"_s) != u"native"_s) && path.value(u"open"_s).toBool());
    }
    m_sameServer->setEnabled(!busy && !managedOpen);
    m_groupServers->setEnabled(!busy && !managedOpen && !m_sameServer->currentData().toString().isEmpty());
    m_resetServerGroups->setEnabled(!busy && !managedOpen && !state.value(u"serverGroups"_s).toObject().isEmpty());
    const QSignalBlocker pathsBlocker(m_paths);
    const QString selectedPath = m_paths->currentItem()
        ? m_paths->currentItem()->data(Qt::UserRole).toString() : QString();
    m_paths->clear();
    for (const QJsonValue &value : state.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        const bool open = path.value(u"open"_s).toBool();
        const bool native = path.value(u"edgeId"_s) == u"native"_s;
        const QString name = path.value(u"proxyName"_s).toString();
        const QJsonObject gateway = path.value(u"gateway"_s).toObject();
        const QString publicEndpoint = gateway.value(u"publicEndpoint"_s).toString();
        QString pathState;
        if (!open)
            pathState = tr("Stopped");
        else if (native)
            pathState = tr("Connected");
        else
            pathState = publicEndpoint.isEmpty() ? tr("Connected, outgoing only") : tr("Connected, public %1").arg(publicEndpoint);
        const QString transport = path.value(u"transport"_s).toObject().value(u"state"_s).toString();
        if (open && (transport == u"checking"))
            pathState += tr("; checking backup connections");
        else if (open && (transport == u"unavailable"))
            pathState += tr("; no reachable backup found");
        else if (open && (transport == u"config-changed"))
            pathState += tr("; settings changed, reconnect to apply");
        auto *item = new QListWidgetItem(u"%1 — %2"_s.arg(native ? tr("Direct connection") : name, pathState), m_paths);
        item->setData(Qt::UserRole, path.value(u"pathId"_s).toString());
        item->setData(Qt::UserRole + 1, open);
        if (item->data(Qt::UserRole).toString() == selectedPath)
            m_paths->setCurrentItem(item);
    }
    if (!m_paths->currentItem() && (m_paths->count() > 0))
        m_paths->setCurrentRow(0);
    m_paths->setVisible(m_paths->count() > 0);
    bool selectedReserve = false;
    const QListWidgetItem *reserve = m_reserves->currentItem();
    if (m_paths->currentItem() && reserve && (reserve->checkState() == Qt::Checked))
    {
        for (const QJsonValue &value : state.value(u"paths"_s).toArray())
        {
            const QJsonObject path = value.toObject();
            if (path.value(u"pathId"_s).toString() == m_paths->currentItem()->data(Qt::UserRole).toString())
                selectedReserve = path.value(u"open"_s).toBool()
                    && path.value(u"reserveNames"_s).toArray().contains(reserve->data(Qt::UserRole).toString());
        }
    }
    m_switch->setEnabled(!busy && selectedReserve);
    m_transportForm->setRowVisible(m_reserves, m_reserves->count() > 0);
    m_transportForm->setRowVisible(m_switch, m_reserves->count() > 0);
    m_url->setEnabled(!busy);
    m_refresh->setEnabled(!busy);
    m_nodeFilter->setEnabled(m_nodes->count() > 0);
    {
        const QSignalBlocker enabledBlocker(m_enabled);
        m_enabled->setChecked(m_manager->managedEnabled());
    }
    if (!busy)
    {
        const QSignalBlocker nodesBlocker(m_nodes);
        const QStringList selected = m_manager->selectedNodes();
        for (int row = 0; row < m_nodes->count(); ++row)
        {
            QListWidgetItem *item = m_nodes->item(row);
            item->setCheckState(selected.contains(item->data(Qt::UserRole).toString())
                ? Qt::Checked : Qt::Unchecked);
        }
    }
    m_nodes->setEnabled(!busy && !m_manager->managedEnabled());
    m_enabled->setEnabled(!busy || m_manager->managedEnabled());
    m_reserves->setEnabled(!busy);
    const QSignalBlocker reservesBlocker(m_reserves);
    for (int row = 0; row < m_reserves->count(); ++row)
    {
        QListWidgetItem *item = m_reserves->item(row);
        item->setFlags(m_manager->managedEnabled()
            ? (item->flags() & ~Qt::ItemIsUserCheckable) : (item->flags() | Qt::ItemIsUserCheckable));
    }
    m_interfaces->setEnabled(!busy && !m_manager->managedEnabled());
    m_dnsServer->setEnabled(!busy);
    m_bootstrapServer->setEnabled(!busy);
    m_dnsFamily->setEnabled(!busy);
    m_dnsApply->setEnabled(!busy);
    m_gatewayControlAddress->setEnabled(!busy);
    m_gatewayDatagramAddress->setEnabled(!busy && m_gatewayUdp->isChecked());
    m_gatewayServerName->setEnabled(!busy);
    m_gatewayCaPath->setEnabled(!busy);
    m_gatewayCertificatePath->setEnabled(!busy);
    m_gatewayPrivateKeyPath->setEnabled(!busy);
    m_gatewayPort->setEnabled(!busy);
    m_gatewayTcp->setEnabled(!busy);
    m_gatewayUdp->setEnabled(!busy);
    m_gatewayApply->setEnabled(!busy);
    m_status->setText(busy ? tr("Working…") : m_manager->status());
    m_status->setVisible(!m_status->text().isEmpty());
}

void PathsWidget::refreshReserves()
{
    const QSignalBlocker reservesBlocker(m_reserves);
    const QString node = selectedNode();
    const QStringList selected = m_manager->reserveNames(node);
    m_reserves->clear();
    const QSignalBlocker groupingBlocker(m_sameServer);
    const QString previousTarget = m_sameServer->currentData().toString();
    m_sameServer->clear();
    m_sameServer->addItem(tr("Choose a node on the same server"), QString());
    const QString serverId = m_nodes->currentItem()
        ? m_nodes->currentItem()->data(Qt::UserRole + 1).toString() : QString();
    if (serverId.isEmpty())
    {
        refreshState();
        return;
    }
    const QString edge = m_manager->edgeIdForServer(serverId);
    for (int index = 0; index < m_nodes->count(); ++index)
    {
        if (index == m_nodes->currentRow())
            continue;
        const QListWidgetItem *candidate = m_nodes->item(index);
        const QString name = candidate->data(Qt::UserRole).toString();
        const QString candidateId = candidate->data(Qt::UserRole + 1).toString();
        if (candidateId.isEmpty())
            continue;
        if (m_manager->edgeIdForServer(candidateId) != edge)
        {
            m_sameServer->addItem(candidate->text(), name);
            continue;
        }
        auto *item = new QListWidgetItem(candidate->text(), m_reserves);
        item->setData(Qt::UserRole, name);
        item->setFlags(item->flags() | Qt::ItemIsUserCheckable);
        item->setCheckState(selected.contains(name) ? Qt::Checked : Qt::Unchecked);
    }
    const int previousIndex = m_sameServer->findData(previousTarget);
    if (previousIndex >= 0)
        m_sameServer->setCurrentIndex(previousIndex);
    refreshState();
}
