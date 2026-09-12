/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include "apicontroller.h"

class PoliciesController final : public APIController
{
    Q_OBJECT
    using APIController::APIController;

private slots:
    void configurationAction();
    void configureAction();
    void previewAction();
    void journalAction();
    void acknowledgeAction();
};
