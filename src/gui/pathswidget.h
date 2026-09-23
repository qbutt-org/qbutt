/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QGroupBox>
#include <QString>

class QCheckBox;
class QComboBox;
class QLabel;
class QLineEdit;
class QPushButton;
class QSpinBox;
class QTreeWidget;

namespace Net
{
    class PathManager;
}

class PathsWidget final : public QGroupBox
{
    Q_OBJECT

public:
    explicit PathsWidget(QWidget *parent = nullptr);

private:
    void refreshState();
    void activateSelection();
    void saveSelection();
    void filterNodes();
    QString selectedServerId() const;
    void loadGatewaySettings();

    Net::PathManager *m_manager;
    QLineEdit *m_url;
    QLineEdit *m_nodeFilter;
    QTreeWidget *m_nodes;
    QCheckBox *m_enabled;
    QComboBox *m_subscriptionFormat;
    QComboBox *m_interfaces;
    QLineEdit *m_dnsServer;
    QLineEdit *m_bootstrapServer;
    QComboBox *m_dnsFamily;
    QPushButton *m_dnsApply;
    QLabel *m_gatewayHeading;
    QLineEdit *m_gatewayControlAddress;
    QLineEdit *m_gatewayDatagramAddress;
    QLineEdit *m_gatewayServerName;
    QLineEdit *m_gatewayCaPath;
    QLineEdit *m_gatewayCertificatePath;
    QLineEdit *m_gatewayPrivateKeyPath;
    QSpinBox *m_gatewayPort;
    QCheckBox *m_gatewayTcp;
    QCheckBox *m_gatewayUdp;
    QPushButton *m_gatewayApply;
    QLabel *m_status;
    bool m_setupIntent = false;
};
