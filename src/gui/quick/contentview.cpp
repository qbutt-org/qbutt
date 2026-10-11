// SPDX-License-Identifier: GPL-2.0-or-later
#include "contentview.h"

#include "qbutt_quick/src/controller.cxxqt.h"

Content::ContentView::ContentView(const QString &profileDir, QWidget *parent)
    : QQuickWidget(parent)
{
    auto *controller = new Qbutt::ContentController;
    controller->setParent(this);
    controller->initLibrary(profileDir);
    controller->setObjectName(QStringLiteral("contentController"));
    connect(controller, &Qbutt::ContentController::torrentReady, this, &ContentView::torrentReady);

    setResizeMode(QQuickWidget::SizeRootObjectToView);
    setInitialProperties({{QStringLiteral("backend"), QVariant::fromValue(controller)}});
    loadFromModule(QStringLiteral("Qbutt.Views"), QStringLiteral("SearchPage"));
}
