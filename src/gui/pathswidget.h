/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QGroupBox>
#include <QString>

class QCheckBox;
class QComboBox;
class QFormLayout;
class QLabel;
class QLineEdit;
class QListWidget;
class QPushButton;
class QSpinBox;

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
    QString selectedNode() const;
    void refreshState();
    void refreshReserves();

    Net::PathManager *m_manager;
    QFormLayout *m_transportForm;
    QLineEdit *m_url;
    QLineEdit *m_nodeFilter;
    QListWidget *m_nodes;
    QCheckBox *m_enabled;
    QComboBox *m_sameServer;
    QPushButton *m_groupServers;
    QPushButton *m_resetServerGroups;
    QListWidget *m_reserves;
    QComboBox *m_interfaces;
    QLineEdit *m_dnsServer;
    QLineEdit *m_bootstrapServer;
    QComboBox *m_dnsFamily;
    QPushButton *m_dnsApply;
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
    QListWidget *m_paths;
    QPushButton *m_refresh;
    QPushButton *m_switch;
    QLabel *m_status;
};
