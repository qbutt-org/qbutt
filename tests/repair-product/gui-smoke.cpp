/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <stdexcept>

#include <QApplication>
#include <QCheckBox>
#include <QComboBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QTableWidget>
#include <QTimer>

#include "base/global.h"
#include "base/logger.h"
#include "base/path.h"
#include "base/preferences.h"
#include "base/profile.h"
#include "base/settingsstorage.h"
#include "gui/fspathedit.h"
#include "gui/repairpreviewdialog.h"

namespace
{
    template <typename T>
    T *required(QObject &parent, const QString &name)
    {
        T *result = parent.findChild<T *>(name);
        if (!result)
            throw std::runtime_error(qPrintable(u"Missing widget: "_s + name));
        return result;
    }

    qint64 labelNumber(const QLabel *label)
    {
        QString value = label->text();
        value.remove(QChar::Space);
        value.remove(QChar(0x00a0));
        bool ok = false;
        const qint64 result = value.toLongLong(&ok);
        return ok ? result : -1;
    }
}

int main(int argc, char **argv)
{
    qputenv("QT_QPA_PLATFORM", QByteArrayLiteral("offscreen"));
    QApplication::setAttribute(Qt::AA_DontUseNativeDialogs);
    QApplication application(argc, argv);
    application.setApplicationName(u"qbutt"_s);
    application.setOrganizationName(u"qBittorrent"_s);
    application.setQuitOnLastWindowClosed(false);
    if (argc != 8)
        return 2;
    const QString torrent = QString::fromLocal8Bit(argv[1]);
    const QString destination = QString::fromLocal8Bit(argv[2]);
    const QString roots = QString::fromLocal8Bit(argv[3]);
    const QString mode = QString::fromLatin1(argv[4]);
    const QString screenshot = QString::fromLocal8Bit(argv[5]);
    const QString output = QString::fromLocal8Bit(argv[6]);
    const QString explicitSource = QString::fromLocal8Bit(argv[7]);

    try
    {
        Logger::initInstance();
        Profile::initInstance(Path(QDir(QFileInfo(output).absolutePath()).filePath(u"profile"_s)), {}, false);
        SettingsStorage::initInstance();
        Preferences::initInstance();
        const int font = QFontDatabase::addApplicationFont(
            QDir(qEnvironmentVariable("WINDIR")).filePath(u"Fonts/segoeui.ttf"_s));
        if (font < 0)
            throw std::runtime_error("Cannot load the Windows font for offscreen rendering");

        RepairPreviewDialog dialog;
        required<FileSystemPathEdit>(dialog, u"repairPreviewTorrent"_s)->setSelectedPath(Path(torrent));
        required<FileSystemPathEdit>(dialog, u"repairPreviewDestination"_s)->setSelectedPath(Path(destination));
        auto *sourceRoots = required<QPlainTextEdit>(dialog, u"repairPreviewRoots"_s);
        sourceRoots->setPlainText(roots);
        auto *operation = required<QComboBox>(dialog, u"repairPreviewMode"_s);
        if (mode == u"inplace")
            operation->setCurrentIndex(1);
        auto *status = required<QLabel>(dialog, u"repairPreviewStatus"_s);
        auto *progress = required<QProgressBar>(dialog, u"repairPreviewProgress"_s);
        auto *analyze = required<QPushButton>(dialog, u"repairPreviewAnalyze"_s);
        auto *apply = required<QPushButton>(dialog, u"repairPreviewApply"_s);
        auto *chooseSource = required<QPushButton>(dialog, u"repairPreviewChooseSource"_s);
        auto *reviewed = required<QCheckBox>(dialog, u"repairPreviewReviewed"_s);
        auto *files = required<QTableWidget>(dialog, u"repairPreviewFiles"_s);
        int heartbeats = 0;
        QTimer heartbeat;
        heartbeat.setInterval(1);
        QObject::connect(&heartbeat, &QTimer::timeout, &dialog, [&] { ++heartbeats; });
        heartbeat.start();

        QTimer filePicker;
        filePicker.setInterval(1);
        QObject::connect(&filePicker, &QTimer::timeout, &dialog, [&]
        {
            for (QWidget *widget : QApplication::topLevelWidgets())
            {
                if (auto *picker = qobject_cast<QFileDialog *>(widget))
                {
                    filePicker.stop();
                    picker->selectFile(explicitSource);
                    QMetaObject::invokeMethod(picker, "accept", Qt::DirectConnection);
                    return;
                }
            }
        });

        QJsonObject evidence {{u"mode"_s, mode}};
        bool mappingChosen = false;
        constexpr int MappingRow = 2;
        QTimer observe;
        observe.setInterval(2);
        QObject::connect(&observe, &QTimer::timeout, &dialog, [&]
        {
            const bool complete = status->text().startsWith(u"Read-only preview complete."_s);
            const bool refused = status->text().startsWith(u"Preview refused:"_s)
                || status->text().startsWith(u"Every source directory"_s)
                || status->text().startsWith(u"Choose an existing"_s);
            if (((mode == u"normal") || (mode == u"inplace")) && complete)
            {
                if ((mode == u"normal") && !mappingChosen && (explicitSource != u"-"))
                {
                    mappingChosen = true;
                    files->selectRow(MappingRow);
                    filePicker.start();
                    chooseSource->click();
                    filePicker.stop();
                    analyze->click();
                    return;
                }
                reviewed->setChecked(true);
                evidence.insert(u"rows"_s, files->rowCount());
                evidence.insert(u"heartbeats"_s, heartbeats);
                evidence.insert(u"progressVisible"_s, progress->isVisible());
                evidence.insert(u"applyEnabled"_s, apply->isEnabled());
                evidence.insert(u"status"_s, status->text());
                evidence.insert(u"candidateText"_s, required<QLabel>(dialog, u"repairPreviewCandidates"_s)->text());
                evidence.insert(u"verifiedText"_s, required<QLabel>(dialog, u"repairPreviewVerified"_s)->text());
                evidence.insert(u"networkText"_s, required<QLabel>(dialog, u"repairPreviewNetwork"_s)->text());
                evidence.insert(u"temporaryText"_s, required<QLabel>(dialog, u"repairPreviewTemporary"_s)->text());
                evidence.insert(u"changed"_s, labelNumber(required<QLabel>(dialog, u"repairPreviewChanged"_s)));
                evidence.insert(u"oversized"_s, labelNumber(required<QLabel>(dialog, u"repairPreviewOversized"_s)));
                evidence.insert(u"explicitMapping"_s, (explicitSource == u"-")
                    || (files->item(MappingRow, 1)->text() == QDir::cleanPath(QDir::fromNativeSeparators(explicitSource))));
                evidence.insert(u"sourceRootsEnabled"_s, sourceRoots->isEnabled());
                evidence.insert(u"firstSource"_s, files->item(0, 1)->text());
                evidence.insert(u"screenshotSaved"_s, dialog.grab().save(screenshot));
                dialog.reject();
            }
            else if ((mode == u"refusal") && refused)
            {
                evidence.insert(u"heartbeats"_s, heartbeats);
                evidence.insert(u"status"_s, status->text());
                evidence.insert(u"applyEnabled"_s, apply->isEnabled());
                dialog.reject();
            }
            else if (refused)
            {
                evidence.insert(u"heartbeats"_s, heartbeats);
                evidence.insert(u"status"_s, status->text());
                evidence.insert(u"unexpectedRefusal"_s, true);
                dialog.reject();
            }
        });
        QObject::connect(&dialog, &QDialog::finished, &application, [&]
        {
            if (mode == u"cancel")
            {
                evidence.insert(u"heartbeats"_s, heartbeats);
                evidence.insert(u"status"_s, status->text());
                evidence.insert(u"closedAfterCancel"_s, true);
            }
            QFile file(output);
            if (file.open(QIODevice::WriteOnly | QIODevice::Truncate))
                file.write(QJsonDocument(evidence).toJson(QJsonDocument::Indented));
            application.quit();
        });
        QTimer timeout;
        timeout.setSingleShot(true);
        timeout.setInterval(30000);
        QObject::connect(&timeout, &QTimer::timeout, &dialog, [&]
        {
            evidence.insert(u"timeout"_s, true);
            dialog.reject();
        });
        timeout.start();
        dialog.show();
        analyze->click();
        if (mode == u"cancel")
            QTimer::singleShot(5, &dialog, &QDialog::reject);
        observe.start();
        const int result = application.exec();
        Preferences::freeInstance();
        SettingsStorage::freeInstance();
        Profile::freeInstance();
        Logger::freeInstance();
        return result;
    }
    catch (const std::exception &error)
    {
        QFile file(output);
        if (file.open(QIODevice::WriteOnly | QIODevice::Truncate))
            file.write(QJsonDocument(QJsonObject {{u"error"_s, QString::fromLocal8Bit(error.what())}}).toJson());
        Preferences::freeInstance();
        SettingsStorage::freeInstance();
        Profile::freeInstance();
        Logger::freeInstance();
        return 1;
    }
}
