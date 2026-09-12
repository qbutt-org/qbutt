// Native GUI integration against an explicitly generated, external profile.
// Arguments: settings file, data directory, source base, fresh output directory.

#include <functional>
#include <stdexcept>

#include <libtorrent/torrent_info.hpp>

#include <QApplication>
#include <QCheckBox>
#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QDirIterator>
#include <QElapsedTimer>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QFontDatabase>
#include <QHelpEvent>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QMap>
#include <QMessageBox>
#include <QPixmap>
#include <QPushButton>
#include <QTableWidget>
#include <QTextDocument>
#include <QThread>
#include <QTimer>
#include <QToolButton>
#include <QToolTip>
#include <QTreeWidget>

#include "base/bittorrent/resumedatastorage.h"
#include "base/global.h"
#include "base/logger.h"
#include "base/profile.h"
#include "gui/fspathedit.h"
#include "gui/profileimportdialog.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 30000))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), label + u" timed out");
    }

    QMap<QString, QString> snapshot(const QString &root)
    {
        QMap<QString, QString> result;
        QDirIterator iterator(root, QDir::Files | QDir::Hidden | QDir::System, QDirIterator::Subdirectories);
        while (iterator.hasNext())
        {
            const QString path = iterator.next();
            const QFileInfo info(path);
            require(!info.isSymLink() && !info.isJunction(), u"GUI fixtures must have ordinary files"_s);
            QFile file(path);
            require(file.open(QIODevice::ReadOnly), u"Cannot snapshot the generated source"_s);
            const QByteArray digest = QCryptographicHash::hash(file.readAll(), QCryptographicHash::Sha256).toHex();
            result.insert(QDir(root).relativeFilePath(path), QString::fromLatin1(digest) + u':'
                + QString::number(info.size()) + u':' + QString::number(info.lastModified().toMSecsSinceEpoch()));
        }
        return result;
    }

    void copyPayload(const QString &source, const QString &destination)
    {
        require(QDir().mkpath(destination), u"Cannot create the mapped fixture directory"_s);
        QDirIterator iterator(source, QDir::Files | QDir::Hidden | QDir::System, QDirIterator::Subdirectories);
        while (iterator.hasNext())
        {
            const QString path = iterator.next();
            const QFileInfo info(path);
            require(!info.isSymLink() && !info.isJunction(), u"Mapped fixtures must have ordinary files"_s);
            const QString target = QDir(destination).filePath(QDir(source).relativeFilePath(path));
            require(QDir().mkpath(QFileInfo(target).absolutePath()) && QFile::copy(path, target), u"Cannot copy the generated payload"_s);
        }
    }

    void choosePath(FileSystemPathEdit *field, const QString &path, const bool directory)
    {
        bool selected = false;
        QTimer selection;
        selection.setSingleShot(true);
        QObject::connect(&selection, &QTimer::timeout, field, [&]
        {
            if (auto *picker = qobject_cast<QFileDialog *>(QApplication::activeModalWidget()))
            {
                if (directory)
                    picker->setDirectory(path);
                else
                    picker->selectFile(path);
                selected = QMetaObject::invokeMethod(picker, "accept", Qt::QueuedConnection);
            }
        });
        selection.start(0);
        QTimer timeout;
        timeout.setSingleShot(true);
        QObject::connect(&timeout, &QTimer::timeout, field, []
        {
            if (auto *picker = qobject_cast<QFileDialog *>(QApplication::activeModalWidget()))
                picker->reject();
        });
        timeout.start(5000);
        field->findChild<QToolButton *>()->click();
        require(selected && (field->selectedPath() == Path(path)), u"The real file picker did not select the fixture"_s);
    }
}

int main(int argc, char **argv)
{
    qputenv("QT_QPA_PLATFORM", QByteArrayLiteral("offscreen"));
    QApplication::setAttribute(Qt::AA_DontUseNativeDialogs);
    QApplication app(argc, argv);
    app.setApplicationName(u"qbutt"_s);
    app.setOrganizationName(u"qBittorrent"_s);
    app.setQuitOnLastWindowClosed(false);
    if (argc != 5)
        return 2;
    const QString settingsFile = QString::fromLocal8Bit(argv[1]);
    const QString sourceData = QString::fromLocal8Bit(argv[2]);
    const QString sourceBase = QString::fromLocal8Bit(argv[3]);
    const QString output = QString::fromLocal8Bit(argv[4]);
    if (QFileInfo::exists(output) || !QDir().mkpath(output))
        return 2;
    Logger::initInstance();
    QJsonObject evidence {{u"status"_s, u"failed"_s}};
    bool profileInitialized = false;
    int exitCode = 1;
    try
    {
#ifdef Q_OS_WIN
        const int font = QFontDatabase::addApplicationFont(QDir(qEnvironmentVariable("WINDIR")).filePath(u"Fonts/segoeui.ttf"_s));
        require(font >= 0, u"Cannot load the Windows font for offscreen rendering"_s);
        app.setFont(QFont(QFontDatabase::applicationFontFamilies(font).constFirst(), 9));
#endif
        require(Path(settingsFile).hasAncestor(Path(sourceBase)) && Path(sourceData).hasAncestor(Path(sourceBase)),
            u"The generated source must be wholly inside the supplied fixture base"_s);
        const auto before = snapshot(sourceBase);
        const auto sourceRecords = BitTorrent::ResumeDataStorage::readExternal(Path(sourceData), Path(sourceBase));
        require(sourceRecords && (sourceRecords->size() == 3), u"A generated three-torrent source is required"_s);
        QMap<QString, QMap<QString, QString>> payloadSnapshots;
        for (const auto &record : *sourceRecords)
        {
            require(record.result.has_value(), u"The generated source contains an invalid record"_s);
            const QString payload = QString::fromStdString(record.result->ltAddTorrentParams.save_path);
            payloadSnapshots.insert(payload, snapshot(payload));
        }
        const QString targetProfile = QDir(output).filePath(u"profile"_s);
        Profile::initInstance(Path(targetProfile), {}, false);
        profileInitialized = true;
        {
            ProfileImportDialog dialog;
            dialog.show();
            QCoreApplication::processEvents();
            auto *settings = dialog.findChild<FileSystemPathEdit *>(u"profileSettingsFile"_s);
            auto *data = dialog.findChild<FileSystemPathEdit *>(u"profileDataDirectory"_s);
            auto *base = dialog.findChild<FileSystemPathEdit *>(u"profileSourceBase"_s);
            auto *preview = dialog.findChild<QPushButton *>(u"profileImportPreview"_s);
            auto *apply = dialog.findChild<QPushButton *>(u"profileImportApply"_s);
            auto *ownership = dialog.findChild<QCheckBox *>(u"profileImportOwnership"_s);
            auto *torrents = dialog.findChild<QTableWidget *>(u"profileImportTorrents"_s);
            auto *settingList = dialog.findChild<QTreeWidget *>(u"profileImportSettings"_s);
            auto *status = dialog.findChild<QLabel *>(u"profileImportStatus"_s);
            require(settings && data && base && preview && apply && ownership && torrents && settingList && status,
                u"The native profile dialog controls are missing"_s);
            require(!preview->isEnabled() && !apply->isEnabled(), u"Empty input enabled an import action"_s);
            require(dialog.grab().save(QDir(output).filePath(u"initial.png"_s)), u"Cannot render the initial dialog"_s);
            choosePath(settings, settingsFile, false);
            choosePath(data, sourceData, true);
            choosePath(base, sourceBase, true);
            require(preview->isEnabled(), u"Valid source selection did not enable preview"_s);
            settings->setSelectedPath(Path(QDir(sourceBase).filePath(u"missing-settings.ini"_s)));
            preview->click();
            waitFor(u"Invalid source preview"_s, [&] { return preview->isEnabled(); });
            require((torrents->rowCount() == 0) && !apply->isEnabled()
                && status->text().startsWith(u"Cannot preview this profile:"_s), u"An invalid source did not return a usable error state"_s);
            choosePath(settings, settingsFile, false);
            bool heartbeat = false;
            QTimer::singleShot(0, &dialog, [&] { heartbeat = true; });
            preview->click();
            require(!preview->isEnabled() && !apply->isEnabled(), u"Preview did not hold editing controls"_s);
            waitFor(u"Async profile preview"_s, [&] { return preview->isEnabled() && heartbeat; });
            require(torrents->rowCount() == 3, status->text());
            bool metadataNameChecked = false;
            for (const auto &record : *sourceRecords)
            {
                if (!record.result->name.isEmpty())
                    continue;
                const QString expectedName = QString::fromStdString(record.result->ltAddTorrentParams.ti->name());
                for (int row = 0; row < torrents->rowCount(); ++row)
                {
                    QTextDocument identity;
                    identity.setHtml(torrents->item(row, 1)->toolTip());
                    if (identity.toPlainText() != record.torrentID.toString())
                        continue;
                    require(torrents->item(row, 1)->text() == expectedName, u"A torrent without a custom name did not show its metadata name"_s);
                    metadataNameChecked = true;
                }
            }
            require(metadataNameChecked, u"The generated source must include a torrent without a custom name"_s);
            require(snapshot(sourceBase) == before, u"Preview changed the source profile or payload"_s);
            for (auto it = payloadSnapshots.cbegin(); it != payloadSnapshots.cend(); ++it)
                require(snapshot(it.key()) == it.value(), u"Preview changed a source payload"_s);
            require(!apply->isEnabled(), u"Preview bypassed explicit ownership consent"_s);
            require(settingList->topLevelItemCount() == 2 && settingList->topLevelItem(0)->childCount() > 0
                && settingList->topLevelItem(1)->childCount() > 0, u"Settings allowlist/exclusion preview is incomplete"_s);
            for (int i = 0; i < settingList->topLevelItem(1)->childCount(); ++i)
                require(settingList->topLevelItem(1)->child(i)->text(1).isEmpty(), u"An excluded setting exposed its value"_s);
            settingList->topLevelItem(1)->setExpanded(true);
            QTextDocument mappingText;
            mappingText.setHtml(torrents->item(0, 2)->toolTip());
            require(mappingText.toPlainText().contains(u"Relative file paths"_s), u"Actual file mappings are missing"_s);
            require(dialog.grab().save(QDir(output).filePath(u"preview.png"_s)), u"Cannot render the profile preview"_s);
            const QPoint mappingCell = torrents->visualItemRect(torrents->item(0, 2)).center();
            QHelpEvent mappingHelp(QEvent::ToolTip, mappingCell, torrents->viewport()->mapToGlobal(mappingCell));
            QCoreApplication::sendEvent(torrents->viewport(), &mappingHelp);
            QWidget *mappingTooltip = nullptr;
            for (QWidget *widget : QApplication::topLevelWidgets())
            {
                if ((widget->windowType() == Qt::ToolTip) && widget->isVisible())
                    mappingTooltip = widget;
            }
            require(mappingTooltip && mappingTooltip->grab().save(QDir(output).filePath(u"mapping-details.png"_s)),
                u"Cannot render the actual mapped-file tooltip"_s);
            QToolTip::hideText();
            ownership->setChecked(true);
            require(apply->isEnabled(), u"Explicit ownership did not enable a valid import"_s);
            for (int row = 0; row < torrents->rowCount(); ++row)
                torrents->item(row, 0)->setCheckState(Qt::Unchecked);
            require(!apply->isEnabled(), u"Import accepted an empty torrent selection"_s);
            torrents->item(0, 0)->setCheckState(Qt::Checked);
            torrents->item(1, 0)->setCheckState(Qt::Checked);
            torrents->item(0, 3)->setText(u"relative/path"_s);
            require(!apply->isEnabled(), u"Import accepted a relative destination"_s);
            const QString mapped = QDir(output).filePath(u"mapped-payload"_s);
            copyPayload(torrents->item(0, 2)->text(), mapped);
            torrents->item(0, 3)->setText(mapped);
            require(apply->isEnabled(), u"A valid explicit mapping was not accepted"_s);
            require(dialog.grab().save(QDir(output).filePath(u"selection.png"_s)), u"Cannot render the selected mapping"_s);
            heartbeat = false;
            QTimer::singleShot(0, &dialog, [&] { heartbeat = true; });
            apply->click();
            require(!dialog.close() && dialog.isVisible(), u"Closing abandoned an active prepare operation"_s);
            waitFor(u"Async profile import preparation"_s, [&]
            {
                return (dialog.findChild<QMessageBox *>() != nullptr) || apply->isEnabled();
            });
            auto *notice = dialog.findChild<QMessageBox *>();
            require(notice && heartbeat, status->text());
            require(notice->text().contains(u"Restart qbutt"_s), u"Successful preparation omitted the restart requirement"_s);
            require(notice->grab().save(QDir(output).filePath(u"prepared.png"_s)), u"Cannot render the preparation notice"_s);
            const Path imports = specialFolderLocation(SpecialFolder::Data) / Path(u"profile-import/imports"_s);
            const auto staged = BitTorrent::ResumeDataStorage::readExternal(imports, Path(targetProfile));
            require(staged && (staged->size() == 2), u"The selected native records were not staged"_s);
            bool mappingPersisted = false;
            for (const auto &record : *staged)
            {
                require(record.result && record.result->stopped, u"Imported native state was not stopped"_s);
                mappingPersisted |= record.result->savePath == Path(mapped);
            }
            require(mappingPersisted, u"The edited destination did not reach native persistence"_s);
            notice->accept();
            waitFor(u"Import dialog acknowledgement"_s, [&] { return !dialog.isVisible(); });
            require(snapshot(sourceBase) == before, u"GUI import changed the source profile or payload"_s);
            for (auto it = payloadSnapshots.cbegin(); it != payloadSnapshots.cend(); ++it)
                require(snapshot(it.key()) == it.value(), u"GUI import changed a source payload"_s);
            evidence = {{u"status"_s, u"passed"_s}, {u"previewTorrents"_s, 3}, {u"selectedTorrents"_s, 2}
                , {u"sourceReadOnly"_s, true}, {u"nativeFilePickersExercisedWithQtTestDialogs"_s, true}
                , {u"consentAndMappingsChecked"_s, true}, {u"asyncEventDispatch"_s, true}
                , {u"invalidSourceErrorRecoverable"_s, true}
                , {u"nativeMetadataNameShown"_s, true}
                , {u"pendingImportCloseRejected"_s, true}, {u"nativeStageReadback"_s, true}
                , {u"targetProfile"_s, targetProfile}, {u"mappedPayload"_s, mapped}};
        }
        exitCode = 0;
    }
    catch (const std::exception &error)
    {
        evidence.insert(u"error"_s, QString::fromUtf8(error.what()));
    }
    if (profileInitialized)
        Profile::freeInstance();
    Logger::freeInstance();
    QFile evidenceFile(QDir(output).filePath(u"evidence.json"_s));
    if (!evidenceFile.open(QIODevice::WriteOnly))
        return 1;
    const QByteArray json = QJsonDocument(evidence).toJson(QJsonDocument::Indented);
    if (evidenceFile.write(json) != json.size())
        return 1;
    return exitCode;
}
