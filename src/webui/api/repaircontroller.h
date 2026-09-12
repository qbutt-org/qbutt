/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#pragma once

#include <QJsonObject>
#include <QTimer>

#include "apicontroller.h"

namespace BitTorrent
{
    class RepairService;
}

class RepairController final : public APIController
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(RepairController)

public:
    RepairController(IApplication *app, QObject *parent);

private slots:
    void analyzeAction();
    void statusAction();
    void applyAction();
    void cancelAction();

private:
    void requireOperation() const;
    void clearOperation();

    BitTorrent::RepairService *m_service = nullptr;
    QJsonObject m_status;
    QTimer m_expiryTimer;
};
