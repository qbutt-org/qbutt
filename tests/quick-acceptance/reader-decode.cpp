// SPDX-License-Identifier: GPL-2.0-or-later
#include <QCoreApplication>
#include <QDebug>
#include <QImage>
#include <QSize>
#include <QStringList>

#include "qbutt_quick/native.h"

int main(int argc, char *argv[])
{
    QCoreApplication application(argc, argv);
    const QStringList arguments = application.arguments();
    if (arguments.size() != 3)
    {
        qCritical("Usage: qbutt-reader-decode-fixture <page-2.png> <corrupt.png>");
        return 1;
    }

    const QImage image = Qbutt::readReaderImage(arguments[1]);
    if (image.size() != QSize(4, 3) || image.pixel(0, 0) != qRgb(0, 157, 247)
        || Qbutt::validatedImageSize(arguments[1]) != image.size()
        || !Qbutt::readReaderImage(arguments[2]).isNull()
        || !Qbutt::validatedImageSize(arguments[2]).isEmpty())
    {
        qCritical("Generated exact page or corrupt-image rejection failed");
        return 1;
    }

    qInfo("PASS production Qt reader decode: exact authored page 2 and corrupt image rejection; no window");
    return 0;
}
