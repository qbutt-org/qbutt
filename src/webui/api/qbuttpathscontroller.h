/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include "apicontroller.h"

class QbuttPathsController final : public APIController
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(QbuttPathsController)

    using APIController::APIController;

private slots:
    void statusAction();
    void listAction();
    void refreshAction();
    void openAction();
    void transportAction();
    void policyAction();
    void dnsAction();
    void gatewayAction();
    void resolveAction();
    void stopAction();
    void nativeAction();

private:
    void requireIdle() const;
};
