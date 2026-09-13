// Generates a synthetic one-second motion fixture. No user media is accessed.
import AVFoundation
import Foundation
let url=URL(fileURLWithPath:CommandLine.arguments[1])
let writer=try AVAssetWriter(outputURL:url,fileType:.mp4)
let input=AVAssetWriterInput(mediaType:.video,outputSettings:[AVVideoCodecKey:AVVideoCodecType.h264,AVVideoWidthKey:128,AVVideoHeightKey:96])
let adaptor=AVAssetWriterInputPixelBufferAdaptor(assetWriterInput:input,sourcePixelBufferAttributes:[kCVPixelBufferPixelFormatTypeKey as String:kCVPixelFormatType_32ARGB,kCVPixelBufferWidthKey as String:128,kCVPixelBufferHeightKey as String:96])
writer.add(input);writer.startWriting();writer.startSession(atSourceTime:.zero)
for frame in 0..<30 {
    while !input.isReadyForMoreMediaData {Thread.sleep(forTimeInterval:0.001)}
    var pixel:CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil,adaptor.pixelBufferPool!,&pixel)==kCVReturnSuccess,let pixel else{fatalError("fixture allocation")}
    CVPixelBufferLockBaseAddress(pixel,[])
    let base=CVPixelBufferGetBaseAddress(pixel)!.assumingMemoryBound(to:UInt8.self)
    let stride=CVPixelBufferGetBytesPerRow(pixel)
    for y in 0..<96{for x in 0..<128{let p=base+y*stride+x*4;p[0]=255;p[1]=UInt8((x+frame*4)%256);p[2]=UInt8(y*2);p[3]=UInt8(frame*8)}}
    CVPixelBufferUnlockBaseAddress(pixel,[])
    guard adaptor.append(pixel,withPresentationTime:CMTime(value:Int64(frame),timescale:30))else{fatalError("fixture encoding")}
}
input.markAsFinished()
let finished=DispatchSemaphore(value:0);writer.finishWriting{finished.signal()};finished.wait()
guard writer.status == .completed else{fatalError("fixture finalization")}
print("Synthetic motion fixture generated")
