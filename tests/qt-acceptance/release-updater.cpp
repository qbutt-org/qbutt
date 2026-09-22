/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <cstdio>
#include <memory>

#include <QApplication>
#include <QFile>
#include <QFontDatabase>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QNetworkProxy>
#include <QPushButton>
#include <QSslCertificate>
#include <QSslConfiguration>
#include <QStandardPaths>
#include <QStyleHints>
#include <QTimer>

#include "base/global.h"
#include "base/releaseupdater.h"
#include "gui/releaseupdatedialog.h"
#include "update-registry.h"

// A real Qt process and real HTTPS sockets. The runner's generated CA is trusted
// only in this test process; installed trust stores and production code are unchanged.
int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    const int font = QFontDatabase::addApplicationFont(qEnvironmentVariable("WINDIR") + u"/Fonts/segoeui.ttf");
    if (font >= 0)
        app.setFont(QFont(QFontDatabase::applicationFontFamilies(font).first(), 9));
    app.setStyle(u"Fusion"_s);
    app.styleHints()->setColorScheme(Qt::ColorScheme::Light);
    if (app.arguments().size() != 3)
        return 2;
    const QString mode = app.arguments()[1];
    const QString target = app.arguments()[2];
    const bool installed = mode.startsWith(u"installed-");
    if (installed)
        app.setApplicationName(qEnvironmentVariable("QBUTT_UPDATE_FIXTURE_CACHE_NAME"));
    std::unique_ptr<UpdateRegistryFixture> registry;
    if (installed)
    {
        registry = std::make_unique<UpdateRegistryFixture>(QCoreApplication::applicationDirPath());
        if (!registry->isReady())
            return 7;
    }
    const QString ca = qEnvironmentVariable("QBUTT_UPDATE_FIXTURE_CA");
    if (!ca.isEmpty())
    {
        const auto certs = QSslCertificate::fromPath(ca);
        if (certs.isEmpty())
            return 3;
        auto config = QSslConfiguration::defaultConfiguration();
        if (mode != u"untrusted")
            config.setCaCertificates(certs);
        QSslConfiguration::setDefaultConfiguration(config);
        QNetworkProxy::setApplicationProxy(QNetworkProxy(QNetworkProxy::HttpProxy, u"127.0.0.1"_s,
            static_cast<quint16>(qEnvironmentVariableIntValue("QBUTT_UPDATE_FIXTURE_PORT"))));
    }
    ReleaseUpdater service;
    auto *dialog = new ReleaseUpdateDialog(&service);
    dialog->show();
    ReleaseUpdater *const updater = &service;
    if (updater->isInstalled() != installed)
        return 4;
    bool done = false;
    bool duplicateCheck = false;
    qint64 receivedBytes = 0;
    const auto complete = [&]()
    {
        if (done)
            return;
        const auto state = updater->state();
        if ((state == ReleaseUpdater::State::Checking) || (state == ReleaseUpdater::State::Downloading))
            return;
        if (state == ReleaseUpdater::State::Available)
        {
            if (installed)
                return;
            if ((mode != u"versions") && (mode != u"live"))
            {
                updater->download(target);
                return;
            }
        }
        if ((state == ReleaseUpdater::State::Ready) && (mode == u"installed-tampered"))
        {
            QFile file(updater->savedPath());
            if (!file.open(QIODevice::ReadWrite) || (file.write("!") != 1))
            {
                app.exit(8);
                return;
            }
            file.close();
            if (updater->install())
                app.exit(9);
            return;
        }
        done = true;
        QTimer::singleShot(0, &app, [&, state]()
        {
            const auto *label = dialog->findChild<QLabel *>(u"releaseStatus"_s);
            const auto *button = dialog->findChild<QPushButton *>(u"releaseDownload"_s);
            const auto *installButton = dialog->findChild<QPushButton *>(u"releaseInstall"_s);
            const bool controls = label && (label->text() == updater->message()) && button
                && (button->isEnabled() == (state == ReleaseUpdater::State::Available))
                && (button->isVisible() == !installed) && installButton
                && (installButton->isVisible() == (installed && (state == ReleaseUpdater::State::Ready)));
            const bool fits = label && (label->height() >= label->heightForWidth(label->width()));
            const QJsonObject result {{u"state"_s, static_cast<int>(state)}, {u"message"_s, updater->message()},
                {u"fileName"_s, updater->fileName()}, {u"receivedBytes"_s, receivedBytes},
                {u"installed"_s, updater->isInstalled()}, {u"savedPath"_s, updater->savedPath()},
                {u"cacheRoot"_s, QStandardPaths::writableLocation(QStandardPaths::CacheLocation)},
                {u"duplicateCheck"_s, duplicateCheck},
                {u"controls"_s, controls}, {u"textFits"_s, fits},
                {u"statusHeight"_s, label ? label->height() : 0},
                {u"requiredStatusHeight"_s, label ? label->heightForWidth(label->width()) : 0}};
            const QByteArray json = QJsonDocument(result).toJson(QJsonDocument::Compact);
            std::puts(json.constData());
            dialog->grab().save(target + u".png");
            const bool downloaded = (mode != u"live-download") || (state == ReleaseUpdater::State::Ready);
            app.exit(controls && fits && downloaded ? 0 : 5);
        });
    };
    QObject::connect(updater, &ReleaseUpdater::changed, &app, complete);
    QObject::connect(updater, &ReleaseUpdater::progress, &app, [&](qint64 received, qint64)
    {
        receivedBytes = received;
        if (((mode == u"cancel") || (mode == u"installed-cancel")) && (received > 0))
            updater->cancel();
        if ((mode == u"installed-duplicate-check") && !duplicateCheck && (received > 0))
        {
            duplicateCheck = true;
            updater->check();
        }
    });
    QTimer::singleShot(65000, &app, [&]() { app.exit(6); });
    return app.exec();
}
